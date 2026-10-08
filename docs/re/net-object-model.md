# Network object model — the wire side

How objects appear in game messages. The in-memory side (Object/Entity hierarchy, World
containers, NetworkId allocation, owner/locality rules, static WRP object ids) is documented in
`docs/re/world-object-model.md` (separate RE task); keep the two consistent.

Arma 3 2.22.0.154103, RVAs of `arma3_x64.exe`.

## Network object id on the wire — high confidence

Item data type 20 (`netid`, primitive `0xc797b0`): two unsigned LEB128 varints

```
var creator    // player/connection id (dpnid) of the machine that created the object
var id         // per-creator serial
```

Type 21 (`netidarr`) = `var count` × netid. Every object reference in a game message uses this
pair; there is no other object addressing on the wire. In memory the pair is `ObjectId`/
`NetworkId` (RTTI: `RemoveSoftLinks<SoftLinkIdTraits<ObjectId,ObjectIdAutoDestroy>>` is a base of
`NetworkObject`). Log formats such as `"Server: Object %d:%d not found (message %s)"`,
`"Unit %d:%d not found, cannot update"` and `"Cannot create object %d:%d with type[%s], param[%s],
NMT code[%d]"` print it as `creator:id`.

The player id (`creator`) of a client is the id assigned in the connect RESULT
(`net-handshake.md`); the server uses its own id for server-created objects.

## Ownership and locality on the wire

- **Owner change**: message **355** `{int, int}` (client → server); the server checks the sender
  owns the object (`"Server: OwnerChanged of %d:%d arrived from non owner %d"`) and looks up its
  `NetworkObjectInfo` (`"Server: Object info %d:%d not found during Changing Owner"`).
- **Updates**: the owning machine sends update messages (about 40 ids share one server case that
  logs `"Unit %d:%d not found, cannot update"`, e.g. 105–156, see
  `net-message-dispatch.tsv`); the server forwards them to other clients. Items carry error
  metrics (`/e<type>:<coef>` in `net-message-formats.tsv`) that drive the update priority.
- Per-player object state: `NetworkObjectInfo` / `NetworkPlayerObjectInfo`
  (`"NetworkObjectInfo::GetPlayerObjectInfo"`, `"Wrong player index passed to %s"`, server case
  0xca7e13 shared by ids 182, 209, 212, 254, 260, 267, 348, 350, 475, 478).

## Create / delete

Object creation messages are reported by the client as
`"Cannot create object %d:%d with type[%s], param[%s], NMT code[%d]: "` (the creation message
id is logged as "NMT code"); deletion by `"Client: local object destroyed (DeleteObject function)
%d:%d"`. Identifying the exact create/update/delete ids and their item layouts per object class
is open (follow-up issue): start from these strings' xrefs in the client dispatcher
(`0xc3ed30`) and from `net-message-formats.tsv` rows whose first items are `netid`.
