//! Authority-signed membership hydration. Labels never authorize membership changes.
use super::*;
pub(super) fn apply(tx: &Transaction<'_>, event: &Event, at: u64) -> Result<bool, StorageError> {
    if !matches!(
        event.event_type,
        EventType::MemberAdded | EventType::DeviceAdded
    ) {
        return Ok(false);
    }
    let authority: String = tx.query_row(
        "SELECT authority_member_id FROM teams WHERE team_id=?1",
        params![event.team_id],
        |row| row.get(0),
    )?;
    if authority != event.actor_member_id {
        return Err(StorageError::UnauthorizedEvent {
            actor_member_id: event.actor_member_id.clone(),
            device_id: event.device_id.clone(),
        });
    }
    let field = if event.event_type == EventType::MemberAdded {
        "member_id"
    } else {
        "device_id"
    };
    let id = event
        .payload
        .get(field)
        .and_then(Value::as_str)
        .ok_or(StorageError::MembershipConflict("membership record ID"))?;
    // Project the same winner on peers regardless of arrival order, preserving
    // stale signed history without reactivating a subsequently revoked device.
    let newest:Option<String>=tx.query_row("SELECT event_id FROM event_log WHERE team_id=?1 AND event_type=?2 AND json_extract(payload_json,?3)=?4 ORDER BY logical_time DESC,event_id DESC LIMIT 1",params![event.team_id,event.event_type.as_str(),format!("$.{field}"),id],|row|row.get(0)).optional()?;
    if newest.as_deref() != Some(event.event_id.as_str()) {
        return Ok(true);
    }
    match event.event_type {
        EventType::MemberAdded => {
            let member: Member = serde_json::from_value(event.payload.clone())?;
            validate_payload_text(&member.member_id, "member.member_id")?;
            validate_payload_text(&member.display_name, "member.display_name")?;
            tx.execute("INSERT INTO team_members(team_id,member_id,display_name,role_json,active,updated_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(team_id,member_id) DO UPDATE SET display_name=excluded.display_name,role_json=excluded.role_json,active=excluded.active,updated_at=excluded.updated_at",params![event.team_id,member.member_id,member.display_name,encode(&member.role)?,bool_to_int(member.active),at])?;
        }
        EventType::DeviceAdded => {
            let device: Device = serde_json::from_value(event.payload.clone())?;
            if DeviceIdentity::device_id_for_public_key(&device.public_key)? != device.device_id {
                return Err(StorageError::MembershipConflict("device public key"));
            }
            tx.query_row(
                "SELECT 1 FROM team_members WHERE team_id=?1 AND member_id=?2",
                params![event.team_id, device.member_id],
                |_| Ok(()),
            )?;
            let previous: Option<(String, String)> = tx
                .query_row(
                    "SELECT member_id,public_key FROM devices WHERE device_id=?1",
                    params![device.device_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if previous
                .is_some_and(|(member, key)| member != device.member_id || key != device.public_key)
            {
                return Err(StorageError::MembershipConflict("device owner"));
            }
            tx.execute("INSERT INTO devices(device_id,member_id,public_key,active,updated_at) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(device_id) DO UPDATE SET active=excluded.active,updated_at=excluded.updated_at",params![device.device_id,device.member_id,device.public_key,bool_to_int(device.active),at])?;
        }
        _ => unreachable!(),
    }
    Ok(true)
}
