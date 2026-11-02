# AMD read-only graph references

Verified 2026-11-02 against AMD's official display-library SDK, commit
`84c881cb2136f04287eb3af12de00a9e2867aa9e` and its generated public header documentation.

- [adapter.h](https://gpuopen-librariesandsdks.github.io/adl/adapter_8h_source.html), line 125: `ADL_Adapter_ObservedGameClockInfo_Get` takes an ADL context, adapter index, and four `int*` outputs in base/game/boost/memory order. Despite the ADL prefix, context is required. Headers declare C linkage and no special calling convention. The observed-clock API family reports MHz; the game-clock entry's generated parameter documentation is incomplete, so a live-driver sanity check remains useful.
- [overdrive8.h](https://gpuopen-librariesandsdks.github.io/adl/overdrive8_8h_source.html), line 321: `ADL2_Adapter_PMLog_SensorLimits_Get` takes context, adapter index, and `ADLPMLogSensorLimits*`.
- SDK `include/adl_structures.h` defines the output solely as `int SensorLimits[ADL_PMLOG_MAX_SENSORS][2]`, min then max, with no leading size field. `include/adl_defines.h` defines 256 sensors. Windows C ints are 32 bits: 2048 bytes, 4-byte alignment.
- Sensor indices: ASIC power 23, board power 73. PMLog power readings and their limits use watts; domain data uses milliwatts. The reference follows whichever sensor supplies the current reading, including when board support changes.

Both exports are optional. Failed queries and invalid values leave the reference unknown. Driver sensor ranges are display scales, not a guarantee of advertised board power or the active power cap. Positive manual settings override the display scale only. No hardware tuning APIs are called.
