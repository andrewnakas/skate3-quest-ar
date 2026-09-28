# Retail audio object registry

Read on 2026-09-20 out of the decrypted retail image (`default_82000000_011B0000.bin`, base
`0x82000000`; see the dump recipe in `engine-defects.md` §9). These three tables are **read, not
inferred** — the class names are the retail strings at each descriptor's `+4`.

Skate 3's audio subsystem is a three-level, data-driven object model:

```
CSTATEMGR_<X>            one per category, held in modules[14] at [g0x830CFDC4 + 656..708]
  └── CSTATE_<X>         N state slots, LRU, linked through [node+4] from [manager+16]
        └── SFXObj_<Y>   components, selected by (category, kind), linked from [state+36]
```

A manager's registry id is written into `[manager+12]` by `vtable[+4]` (`sub_828DE848`, body
`stw r4,12(r3)`), and that id is the **category** that selects the state class and the components.
Each descriptor's word 0 packs `category<<16 | kind<<4 | ctor-arg`; `+4` is the name string and
`+12` (managers: `+8`) is the create function.

The three vectors are pushed at startup from `skate3_recomp.9.cpp`: managers by `sub_8248D2A0`,
states by `sub_828DE8C0`, components by `sub_828DE928`.

| id | manager (0x830BBE00) | create |
|---:|---|---|
| 0 | `CSTATEMGR_Main` | `sub_824F1C60` (desc `0x8302CD98`) |
| 1 | `CSTATEMGR_Player` | `sub_824F1E40` (desc `0x8302CDA4`) |
| 2 | `CSTATEMGR_Environment` | `sub_824F1A88` (desc `0x8302CD8C`) |
| 3 | `CSTATEMGR_Collision` | `sub_824F16D0` (desc `0x8302CD1C`) |
| 4 | `CSTATEMGR_TrafficCar` | `sub_824F22F8` (desc `0x8302CDB0`) |
| 5 | `CSTATEMGR_Pedestrian` | `sub_824F2740` (desc `0x8302CDBC`) |
| 6 | `CSTATEMGR_Emitter` | `sub_824F2F98` (desc `0x8302CDC8`) |
| 7 | `CSTATEMGR_Crowd` | `sub_824F30B0` (desc `0x8302CDD4`) |
| 8 | `CSTATEMGR_DynamicObject` | `sub_824F3D18` (desc `0x8302CDE0`) |
| 9 | `CSTATEMGR_Speaker` | `sub_824F6090` (desc `0x8302CDEC`) |
| 10 | `CSTATEMGR_Whoosh` | `sub_824F18F0` (desc `0x8302CD80`) |
| 11 | `CSTATEMGR_ObjectInstance` | `sub_824F6938` (desc `0x8302CDF8`) |
| 12 | `CSTATEMGR_NISCharacter` | `sub_824F6E30` (desc `0x8302CE04`) |
| 13 | `CSTATEMGR_SkaterSpeech` | `sub_824F7AD8` (desc `0x8302CE10`) |

| cat | state (0x830BBE20) | create |
|---:|---|---|
| 0 | `CSTATE_Main` | `sub_824F8CA0` (desc `0x8302CE4C`) |
| 1 | `CSTATE_Player` | `sub_824F8D60` (desc `0x8302CE5C`) |
| 2 | `CSTATE_Environment` | `sub_824F8BE0` (desc `0x8302CE3C`) |
| 3 | `CSTATE_Collision` | `sub_824F8808` (desc `0x8302CE1C`) |
| 4 | `CSTATE_TrafficCar` | `sub_824F8FA8` (desc `0x8302CE6C`) |
| 5 | `CSTATE_Pedestrian` | `sub_824F90A0` (desc `0x8302CE7C`) |
| 6 | `CSTATE_Emitter` | `sub_824F9380` (desc `0x8302CE8C`) |
| 7 | `CSTATE_Crowd` | `sub_824F9598` (desc `0x8302CE9C`) |
| 8 | `CSTATE_DynamicObject` | `sub_824F9678` (desc `0x8302CEAC`) |
| 9 | `CSTATE_Speaker` | `sub_824F9790` (desc `0x8302CEBC`) |
| 10 | `CSTATE_Whoosh` | `sub_824F89C8` (desc `0x8302CE2C`) |
| 11 | `CSTATE_ObjectInstance` | `sub_824F9AA8` (desc `0x8302CECC`) |
| 12 | `CSTATE_NISCharacter` | `sub_824F9C50` (desc `0x8302CEDC`) |
| 13 | `CSTATE_SkaterSpeech` | `sub_824F9E48` (desc `0x8302CEEC`) |

| cat | kind | component (0x830BBE10) | create |
|---:|---:|---|---|
| 0 | 0 | `SFXObj_Announcer` | `sub_824CF040` (desc `0x8302D130`) |
| 0 | 1 | `SFXObj_Music` | `sub_824D1048` (desc `0x8302D150`) |
| 0 | 2 | `SFXObj_Master` | `sub_824D4F60` (desc `0x8302D190`) |
| 0 | 3 | `SFXObj_CameraMan` | `sub_824D0518` (desc `0x8302D140`) |
| 0 | 5 | `SFXObj_Reverb` | `sub_824DDC98` (desc `0x8302D240`) |
| 0 | 6 | `SFXObj_NIS` | `sub_824E1058` (desc `0x8302D260`) |
| 0 | 7 | `SFXObj_Pause` | `sub_824E1B30` (desc `0x8302D270`) |
| 0 | 8 | `SFXObj_Speech` | `sub_824E1F50` (desc `0x8302D280`) |
| 0 | 9 | `SFXObj_Bloom` | `sub_824ED640` (desc `0x8302D310`) |
| 0 | 10 | `SFXObj_VU` | `sub_824EDAD8` (desc `0x8302D320`) |
| 0 | 11 | `SFXObj_Challenge` | `sub_824EDFD0` (desc `0x8302D330`) |
| 0 | 12 | `SFXObj_HOM` | `sub_824EE610` (desc `0x8302D340`) |
| 0 | 13 | `SFXObj_Menu` | `sub_824EE9E8` (desc `0x8302D350`) |
| 0 | 14 | `SFXObj_Jitter` | `sub_824EEF20` (desc `0x8302D360`) |
| 1 | 0 | `SFXCTL_PlayerPhysics` | `sub_824B0978` (desc `0x8302CF90`) |
| 1 | 0 | `SFXObj_SkateBoard` | `sub_824C4FC8` (desc `0x8302D100`) |
| 1 | 1 | `SFXCTL_3DPlayerPos` | `sub_824B07A8` (desc `0x8302CF70`) |
| 1 | 1 | `SFXObj_Contacts` | `sub_824B7C18` (desc `0x8302D0D0`) |
| 1 | 2 | `SFXCTL_3DBoardPos` | `sub_824B0890` (desc `0x8302CF80`) |
| 1 | 2 | `SFXObj_Wheels` | `sub_824CD668` (desc `0x8302D120`) |
| 1 | 3 | `SFXObj_Rail` | `sub_824C2608` (desc `0x8302D0F0`) |
| 1 | 4 | `SFXObj_Cracks` | `sub_824C1108` (desc `0x8302D0E0`) |
| 1 | 5 | `SFXObj_Tricks` | `sub_824CBD08` (desc `0x8302D110`) |
| 1 | 6 | `SFXObj_Clothing` | `sub_824DB908` (desc `0x8302D210`) |
| 1 | 7 | `SFXObj_Treatments` | `sub_824DD278` (desc `0x8302D230`) |
| 1 | 8 | `SFXObj_SenseOfSpeed` | `sub_824E75C8` (desc `0x8302D2B0`) |
| 1 | 9 | `SFXObj_OffBoard` | `sub_824E8F08` (desc `0x8302D2D0`) |
| 1 | 10 | `SFXObj_HandGrabs` | `sub_824EC1A0` (desc `0x8302D2E0`) |
| 1 | 11 | `SFXObj_Takedown` | `sub_824EF630` (desc `0x8302D370`) |
| 1 | 12 | `SFXObj_DropIn` | `sub_824F03B8` (desc `0x8302D380`) |
| 2 | 0 | `SFXObj_Ambience` | `sub_824D3010` (desc `0x8302D180`) |
| 3 | 0 | `SFXCTL_3DColPos` | `sub_824B36B8` (desc `0x8302D000`) |
| 3 | 0 | `SFXObj_Collision` | `sub_824D1B48` (desc `0x8302D160`) |
| 4 | 0 | `SFXCTL_TrafficCarPhysics` | `sub_824B2770` (desc `0x8302CFC0`) |
| 4 | 0 | `SFXObj_TrafficEngine` | `sub_824D5E18` (desc `0x8302D1A0`) |
| 4 | 1 | `SFXObj_TrafficSkids` | `sub_824D7440` (desc `0x8302D1C0`) |
| 4 | 2 | `SFXObj_TrafficHorn` | `sub_824D6BE8` (desc `0x8302D1B0`) |
| 4 | 3 | `SFXObj_TrafficWoosh` | `sub_824D79C0` (desc `0x8302D1D0`) |
| 5 | 0 | `SFXCTL_PedestrianPhysics` | `sub_824B2FC8` (desc `0x8302CFE0`) |
| 5 | 0 | `SFXObj_PedestrianSpeech` | `sub_824D9048` (desc `0x8302D1F0`) |
| 5 | 1 | `SFXObj_PedestrianSFX` | `sub_824D7E00` (desc `0x8302D1E0`) |
| 5 | 2 | `SFXObj_PedBodyFall` | `sub_824F0880` (desc `0x8302D390`) |
| 5 | 3 | `SFXObj_Tazer` | `sub_824F12D0` (desc `0x8302D3A0`) |
| 6 | 0 | `SFXObj_Emitter` | `sub_824DCBF8` (desc `0x8302D220`) |
| 7 | 0 | `SFXObj_Crowd` | `sub_824DFE80` (desc `0x8302D250`) |
| 8 | 0 | `SFXObj_Dynamic` | `sub_824E2408` (desc `0x8302D290`) |
| 8 | 1 | `SFXObj_Moving` | `sub_824E46B0` (desc `0x8302D2A0`) |
| 9 | 0 | `SFXObj_Speaker` | `sub_824E8650` (desc `0x8302D2C0`) |
| 10 | 0 | `SFXObj_Whoosh` | `sub_824D2918` (desc `0x8302D170`) |
| 11 | 0 | `SFXObj_ObjectInstance` | `sub_824ED0A0` (desc `0x8302D2F0`) |
| 12 | 0 | `SFXObj_NISCharacter` | `sub_824ED3D0` (desc `0x8302D300`) |
| 13 | 0 | `SFXObj_PlayerSpeech` | `sub_824D9F30` (desc `0x8302D200`) |