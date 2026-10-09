# Vehicle physics: cars and tanks (PhysX), helicopters, planes (summary)

This is a summary of how Arma 3 2.22 simulates vehicles, to plan our rapier3d-based
reimplementation. Each section lists:
- what config drives the model;
- what engine code reads it;
- what the code does where it was decoded.

Per-issue follow-ups (#123, #125, #126) should go deeper. Source: `arma3_x64.exe` (RVAs below)
and the shipped config. Class names come from `world-object-model.md`.

## 1. Which model runs (high)

| `simulation` | class | model | shipped classes |
|---|---|---|---|
| `carx` | `CarEPE` | PhysX Vehicle SDK, 4-wheel/N-wheel drive | 359 |
| `tankx` | `TankEPE` | PhysX Vehicle SDK, tank drive | 194 |
| `shipx`, `submarinex`, `hovercraftx` | `ShipEPE`, … | PhysX rigid body plus engine forces | |
| `helicopterrtd` | `HelicopterRTD` | **two models**: basic (engine) or RotorLib advanced flight model | 133 (all helicopters) |
| `airplanex` | `AirplaneEPE` | engine aero model on a PhysX rigid body | 82 |
| `car`, `tank`, `helicopter`, `airplane` | legacy non-PhysX | | none shipped |

"EPE" classes run on PhysX 3.x. The executable contains the PhysX Vehicle SDK field names
(`mTorqueCurve`, `mPeakTorque`, `mMaxOmega`, `mSprungMass`, `mLatStiffX`, `mFrictionVsSlipGraph`,
…), so the config maps directly onto `PxVehicle*Data` structs.

## 2. Cars and tanks (`carx`/`tankx`, loader `0x140f383b0`, high on names/defaults)

**Wheels** (`class Wheels`, one subclass per wheel) → `PxVehicleWheelData`, `PxVehicleSuspensionData`,
`PxVehicleTireData`:

| entries | notes |
|---|---|
| `boneName`, `center`, `boundary`, `width`, `steering` | geometry from memory-LOD points. Wheel mass and MOI are read elsewhere (not traced) |
| `dampingRate`, `dampingRateDamaged`, `dampingRateDestroyed`, `dampingRateInAir` | |
| `maxBrakeTorque`, `maxHandBrakeTorque` | |
| `suspTravelDirection` | default (0, −1, 0) |
| `suspForceAppPointOffset`, `tireForceAppPointOffset` | memory points |
| `maxCompression`, `maxDroop`, `sprungMass`, `springStrength`, `springDamperRate` | |
| `longitudinalStiffnessPerUnitGravity` | |
| `latStiffX` | default 25 |
| `latStiffY` | default 180 |
| `frictionVsSlipGraph[]` | PhysX tire friction curve |
| `wheelDamageThreshold`, `wheelDestroyThreshold`, `…RadiusCoef`, `disableWheelsWhenDestroyed` | wheel hit points |

**Engine** → `PxVehicleEngineData`:

| entry | default / notes |
|---|---|
| `enginePower` | kW, default 50 |
| `maxOmega` | rad/s, default 600 |
| `minOmega` | |
| `engineMOI` | |
| `peakTorque` | N·m. If missing: `peakTorque = enginePower·7040.2144 / (maxOmega·9.549296) · 1.3558179`, i.e. power → torque at max RPM, in lb·ft then converted to N·m |
| `torqueCurve[] = {{ω/ωmax, T/Tpeak}, …}` | up to 8 points, y clamped ≤ 1 |
| `dampingRateFullThrottle` | default 0.08 |
| `dampingRateZeroThrottleClutchEngaged`, `…Disengaged` | |

**Clutch/gears**: `clutchStrength`, `changeGearType` (`rpmratio`), `changeGearOmegaRatios[]`
(pairs), `changeGearMinEffectivity[]`, `switchTime`, `latency`. Gear ratios come from the
`complexGearbox` class (`GearboxRatios[] = {"R1", -ratio, "N", 0, "D1", ratio, …}`; the
conversion has not been traced).
Losses and brakes: `engineBrakeCoef`, `overSpeedBrakeCoef`, `brakeIdleSpeed`, `engineLosses`,
`transmissionLosses`, `useNABrakes`.

**Differential** (cars) → `PxVehicleDifferential4WData`:

| entry | values |
|---|---|
| `differentialType` | `all_open`, `all_limited`, `front_open`, `front_limited`, `rear_open`, `rear_limited` |
| `frontRearSplit` | default 0.5 |
| `frontBias`, `rearBias`, `centreBias` | |

Plus `antiRollbarForceCoef` and the helpers `accelAidForceCoef`, `accelAidForceSpd`,
`accelAidForceYOffset`.

**Tanks** use tank drive: `tankTurnForce`, `tankTurnForceAngMinSpd`, `tankTurnForceAngSpd`. The
tracks are modelled as wheels on each side, driven by a differential thrust.

**Reimplementation note.** rapier3d has no PhysX-equivalent vehicle model. The fields above are
exactly the inputs of PhysX's 4W/tank drive. That drive is documented publicly:
- engine ω integration with a torque curve;
- clutch as a spring between engine and gearbox;
- gearbox with automatic shifting by `changeGearOmegaRatios`;
- limited-slip differential;
- raycast suspension (`springStrength`, `springDamperRate`, `maxCompression/Droop`, `sprungMass`);
- the PhysX tire model (`latStiffX/Y` lateral stiffness vs load,
  `longitudinalStiffnessPerUnitGravity`, `frictionVsSlipGraph`, friction from the bisurf).

Implementing that model on a rapier rigid body with raycast wheels reproduces the config's
meaning.

## 3. Helicopters (`helicopterrtd`)

Two flight models, chosen per player setting (`forceRotorLibSimulation` / `ForceRotorLibSimulation`
can force it):

- **Advanced (RotorLib FDM).** A licensed third-party library. The vehicle's `RTDconfig` points
  to an XML file (e.g. `A3\Air_F\Heli_Light_01\RTD_Heli_Light_01.xml`) with blade, rotor, engine
  and fuselage data. Script access goes through `*RTD` commands (`enginesRpmRTD`,
  `rotorsForcesRTD`, …). This is **not reimplementable 1:1**; a clean-room rotor model would be
  a separate project.
- **Basic (engine model, loader `0x140da39c0`).** Fields:
  - rotors: `mainRotorSpeed`, `backRotorSpeed`, `startDuration`, `mainBladeRadius`,
    `tailBladeRadius`, `tailBladeVertical`;
  - forces: `liftForceCoef`, `cyclicAsideForceCoef`, `cyclicForwardForceCoef`,
    `backRotorForceCoef`, `bodyFrictionCoef`;
  - altitude: `altFullForce`, `altNoForce` (lift fades between them);
  - rotor dive: `min/max/neutralMainRotorDive`, `min/max/neutralBackRotorDive`;
  - `envelope[]`;
  - effects: `washDownStrength`/`Diameter`, gear, sling load limits.

  Example (`B_Heli_Light_01_F`): `liftForceCoef = 1.5`, `bodyFrictionCoef = 0.3`. **Recommended
  target for us.** The force formulas are decoded in `sim-air.md` §2.

## 4. Planes (`airplanex`, loader `0x140d1df50`)

| group | fields |
|---|---|
| Lift | `envelope[]` (lift coefficient table over speed), `angleOfIndicence`, `landingAoa`, `landingSpeed`, `stallSpeedForced`, `stallWarningTreshold` |
| Thrust | `thrustCoef[]` (per speed bin), `throttleToThrustLogFactor` |
| Controls | `elevatorCoef[]`, `aileronCoef[]`, `rudderCoef[]` (per speed bin); `*ControlsSensitivityCoef`; `aileronSensitivity`, `elevatorSensitivity`, `wheelSteeringSensitivity`, `rudderInfluence` |
| Drag | `airFrictionCoefs0/1/2[]` (per-axis constant, linear and quadratic), `flapsFrictionCoef`, `gearsUpFrictionCoef`, `airBrakeFrictionCoef`, `airBrake` |
| Stability ("draconic" forces) | `draconicForceXCoef`, `draconicForceYCoef`, `draconicForceZCoef`, `draconicTorqueXCoef`, `draconicTorqueYCoef`. These turn the velocity toward the nose (weathervane) |
| VTOL | `VTOLYawInfluence`, `VTOLPitchInfluence`, `VTOLRollInfluence` |
| Altitude | `altFullForce`, `altNoForce` |
| Misc | gear/cabin/tail-hook timings, `ejectSpeed`, `ejectDamageLimit` |

The arrays are sampled by airspeed. The bin spacing and the force formulas are decoded in
`sim-air.md` §3. The model is a per-axis coefficient model, not
a blade-element model, so it can be reimplemented on a rapier rigid body with custom forces once
the formulas are pinned down.

## 5. Open points

- Plane aero formulas, envelope bin spacing, basic helicopter forces: done, `sim-air.md`.
- Confirm the PhysX drive mapping by reading the `PxVehicle*Data` setup (strings around
  `0x141d39118`), especially gear ratios and how `complexGearbox` is converted.
- Ship/submarine/hovercraft forces.
