# RTTI class hierarchy (arma3_x64.exe 2.22.0.154103)

The binary keeps full MSVC RTTI. `docs/re/rtti-classes.tsv` lists every class; regenerate it with

```sh
python tools/re/rtti_dump.py P:/a3-rust/oirignal/arma3_x64.exe --tsv docs/re/rtti-classes.tsv
```

Query it with `python tools/re/a3re.py rtti <regex>` / `a3re.py vtable <class>`.

## TSV columns

| Column | Meaning |
|---|---|
| `class` | demangled name (`Bar::Foo`, templates with arguments). 41 complex lambda/template names stay mangled. |
| `mangled` | raw TypeDescriptor name (`.?AVMan@@`) |
| `vtable_rvas` | every vtable whose CompleteObjectLocator points at this class, as `rva` or `rva@offset`, where `offset` is the sub-object offset inside the complete object (multiple inheritance). Add 0x140000000 for the VA. |
| `vtable_slots` | number of consecutive `.text` pointers after each vtable (an upper bound: may run into an adjacent vtable when no COL separates them) |
| `direct_bases` | direct base classes, in declaration order (first = primary base) |
| `all_bases` | every base class, depth-first, as stored in the ClassHierarchyDescriptor |
| `flags` | `MI` multiple inheritance, `VI` virtual inheritance, `no-col` = type descriptor without a vtable/COL (seen via `typeid`/exception tables only) |

Method: TypeDescriptors (`.?AV…`/`.?AU…`) in `.data`; CompleteObjectLocators (signature 1, self-RVA)
in `.rdata`; vtables = the `.rdata` qword equal to the COL VA, plus 8. Confidence: high; the base
lists come straight from the compiler's ClassHierarchyDescriptors.

## Counts

| | |
|---|---|
| Type descriptors | 5,179 |
| With at least one vtable | 4,673 |
| `no-col` | 506 |
| Multiple inheritance (`MI`) | 1,715 |
| Virtual inheritance (`VI`) | 42 |
| Namespaced (`::`) | 1,584 (`DX11::`, `Editor3D::`, `Sound::`, `Steam::`, `Botan::`, `std::`, `enf::`, `RTD::`, ...) |

Ghidra's RTTI analyzer applied the same data to the project, so vtables and `Class::vftable`
labels appear in decompiled code (for example `QIStrStream::vftable`).

## World object model (`Object` → `Entity` → vehicles)

Primary-base tree; the numbers are descendant counts.

```
NetworkObject (172)                      every replicated thing derives from this
├── Object (154)                         placed instance with a shape (LODShape) and position
│   ├── ObjectPlain, ForestPlain, GroundClutter, HeadObject, DroppedWeaponObject, PointLight
│   ├── ObjectColored (4), ObjectWindAnim (4: ObjectWindAnimPar<...Tree/Bush...>), Road (1: TerrainDecal)
│   └── ObjectTyped (135)                Object with an EntityType (config class)
│       ├── MagazineObject, ProxyRetex, ProxyPylonPod (4), WeaponObject (4)
│       └── Entity (122)                 simulated object; also derives Animated
│           ├── Shot (17): Missile, ShotLaser, Mine (BoundingMine, DirectionalBomb),
│           │              ShotShell (ShotBullet, ShotSpread, ShotEPE, Flare, SmokeShell, ...)
│           ├── Smoke (Cloudlet, Crater, Slop), Explosion, Mark, TrackStep, Windlet, ThunderBolt
│           ├── CameraHolder (CameraVehicle, CameraConstruct→CameraCurator, SeaGull), CameraViewer
│           ├── ControlObject (Compass, Watch, Notepad, CHead ...), Detector (triggers), Flag
│           ├── CParticle, CParticleSource, DynSoundSource, SoundOnVehicle, StreetLamp, WindSock
│           ├── EntitySimple, ProxyCrew, ProxySubpart, RopeSegment, ObjectDestructed
│           └── EntityAI (49)            has AI targeting, damage, inventory; + JoinedObject
│               ├── Building (AirportObject, Church, Fountain), Thing (ThingEPE), Fireplace,
│               │   FlagCarrier, RopeObject, VASILights, LaserTarget, NVMarkerTarget, ...
│               └── EntityAIFull (34)
│                   ├── Person (8)
│                   │   ├── Man → Soldier → UavPilot
│                   │   ├── Animal → AnimalSoldier
│                   │   └── InvisibleVehicle → CuratorCommander, HeadlessClientLogic
│                   └── Transport (24)   vehicle with crew positions
│                       ├── TankOrCar (10): Car→CarEPE, Motorcycle, Tank→TankWithAI→TankEPE,
│                       │                   Ship→ShipWithAI→ShipEPE
│                       ├── PlaneOrHeli (7): Airplane→AirplaneAuto→AirplaneAutoEPE,
│                       │                    Helicopter→HelicopterAuto→HelicopterAutoEPE→HelicopterRTD
│                       ├── Parachute→ParachuteAuto, Paraglide→ParaglideAuto
│                       └── Door
├── AI (6): AIBrain (AIUnit, AIAgent), AIGroup, AISubgroup, AICenter
├── AITeam, Command, Turret, Weather, ClientInfoObject, ClientCameraPositionObject,
├── PlayerRuntimeObject, AIStatsMPRow, NetworkAndIdObject→JointConnection
```

Mapping to config `simulation` values (from class names; to confirm in Phase 4): `soldier`→Soldier,
`carx`→CarEPE, `tankx`→TankEPE, `shipx`→ShipEPE, `helicopterrtd`→HelicopterRTD,
`helicopterx`→HelicopterAutoEPE, `airplanex`→AirplaneAutoEPE, `thingx`→ThingEPE,
`house`→Building, `parachute`→ParachuteAuto. The `*EPE` classes are the PhysX-driven variants.

Notable multiple bases (from `direct_bases`):

| Class | Direct bases |
|---|---|
| `Object` | NetworkObject, CountInstances<Object>, ... |
| `Entity` | ObjectTyped, Animated |
| `EntityAI` | Entity, JoinedObject, ... |
| `Person` | EntityAIFull, CountInstances<Person>, ... |
| `Display` | ControlsContainer, AbstractDisplay ... |
| `Control` | RemoveLLinks, IControl |
| `GameValue` | SerializeClass |

## Script values (`GameData` — SQF types)

`GameValue` holds a ref-counted `GameData*`. One `GameData` subclass per SQF type (32):

`GameDataArray, GameDataBool (→ GameDataIf), GameDataCode, GameDataConfig, GameDataControl,
GameDataDiaryRecord, GameDataDisplay, GameDataException, GameDataExpression, GameDataForClass,
GameDataGroup, GameDataHashMap, GameDataLocation, GameDataNaN, GameDataNamespace,
GameDataNetObject, GameDataNil, GameDataNothing, GameDataObject, GameDataScalar, GameDataScript,
GameDataSide, GameDataString, GameDataSubgroup, GameDataSwitch, GameDataTarget, GameDataTask,
GameDataTeamMember, GameDataText, GameDataWhile, GameDataWith`

These match the 35 `GameType` names registered at start-up (see `sqf-command-table.md`). The
compiled-script VM appears as `ScriptSimpleVM::*` (CompileContext, BuildExecutionGraph) and
`SQFBytecode::ScriptSerializer`.

## UI (`ControlsContainer` / `Control`)

- `ControlsContainer` (157): `Display` (142 displays: main menu, editor, map, inventory, MP
  lobby `DisplayServer`, `DisplayVoiceChat`, ...), `MsgBox` (10), `DisplayFileSelect`,
  `MultiplayerSetupMessage`, `ProgressMessage`.
- `Control` (93) for 2D controls: CStatic (19), CListBoxContainer (15), CTree (6),
  CStructuredText (6), CControlsGroup (5), ButtonBase (5), CSliderContainer (4), CEdit, CHTML,
  CWebBrowser, CMenuStrip, CToolBox, CProgressBar, ... 104 classes implement `IControl`
  (includes 3D `ControlObject`s).
- `Editor3D::*` holds the Eden editor; `DisplayCurator` the Zeus UI.

## AI

`AIBrain` (AIUnit, AIAgent), `AIGroup`, `AISubgroup`, `AICenter`, `AITeam`, `Command`,
`AbstractAIMachine` (FSM runner), `AIBrainType`/`AIBrainTypeBank`. All AI containers derive from
`NetworkObject`, so they are replicated (groups and centres exist on every machine).

## Networking

`NetworkManager`, `NetworkComponent` → `NetworkClient`, `NetworkServer`; transport
`NetPeer` → `NetPeerUDP`, `NetPeerToPeerChannel`, `NetChannel`/`NetChannelBasic`,
`NetClient`/`NetClientBase`, `NetServer`/`NetServerBase`, `NetSessionEnum` (server browser,
DNS lookup lambdas). Messages: `NetworkMessage`, `NetworkMessageFormat(Base)`,
`NetworkMessageQueue(Item)`, `NetworkMessageNetworkObject`, `NetworkMessageMessageWithTarget`,
`NetworkMessageVMessage`, `NetworkMessageWithSimulationTime`; per-object info
`NetworkObjectInfo`, `NetworkPlayerInfo`, `NetworkPlayerObjectInfo`, `NetworkSimpleObject`.
Voice: `VoNServer`, `VoNClient`, `VoNSystem*`, `VoNCodec`, `VoiceServer`. Steam:
`Steam::ServerRulesService`, `Steam::SteamMatchmaking`, `SteamRelayRouter`,
`CCallback<NetworkServer,SteamServersConnected_t>`. BattlEye: `BattlEyeNetClient`,
`BattlEyeNetServer`. Detailed protocol notes: `docs/re/net-*.md`.

## Files, serialization, banks

- Streams: `QIStream`, `QOStream`, `QIFStream(B)`, `QIStreamBuffer*` (mapped/paged/temp),
  `QIStrStream`.
- PBO access: `QFBank` ("bank" = mounted PBO), `QFBankFunctions`, `FilebankLoader`,
  `BankSignatureCheckAsync`, `BankHashesCalculatorThread`, `IBankChecker` (bisign checks),
  `FileServer`, `FileServerST`, `FileServerAsync`.
- Config: `ParamArchive`, `ParamArchiveLoad`, `ParamArchiveSave` (save games/mission state),
  `SerializeClass` (148 serialisable classes).
- Resource banks: `ShapeBank`, `TexMaterialBank`, `VehicleTypeBank`, `EntitySimpleTypeBank`,
  `AIBrainTypeBank`, `Sound::*Bank`.

## Other namespaces worth knowing

`DX11::*` (renderer, post-process effects `PPEParsInterpolator<...>`), `Sound::*` (XAudio2
mixer), `RTD::*` (RotorLib helicopter model), `EPERagDoll*` / `*Physx3*` (PhysX integration),
`Botan::*` (crypto), `rapidjson::*`.
