# Team networking notes

Herdr's team discovery uses mDNS service type `_team-assistant._tcp.local.`
and publishes only bounded team/device metadata needed to begin a peer
connection. mDNS is discovery only: the peer handshake still verifies the
advertised device key, active membership, team ID, and signed hello before any
event is accepted.

The local firewall must allow UDP multicast on port 5353 for mDNS and inbound
TCP on the configured Herdr peer port. Managed Wi-Fi networks may isolate
clients or suppress multicast; in that case discovery can return no peers even
when direct connectivity is possible. Use the explicit manual peer address
fallback (`IP:port`) and let the authenticated handshake decide whether the
peer is trusted. A discovered display/member label is never sufficient for
authorization.

The automated tests verify signed advertisement construction, tamper
rejection, and manual address parsing. They do not claim a physical LAN or
firewall test because that requires two running devices on the same network.

For teammates on different networks, configure the optional Cloudflare relay
and run `herdr relay sync`; see [internet team prototype](internet-team.md).
