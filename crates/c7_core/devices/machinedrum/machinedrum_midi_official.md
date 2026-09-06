# Machinedrum MIDI implementation

`machinedrum_manual_OS1.63.pdf`: Pages 113 - 114

---

All machine parameters in the Machinedrum are accessible by MIDI control change messages. This appendix lists the available MIDI CTRL-change mappings. The default MIDI NOTE ON mappings are also listed, although these can be changed using the internal MIDI map editor.

## 1. Default MIDI Mappings

### Note → Track

| Hex | Dec | Note | Track |
| --- | --- | ---- | ----- |
| 24  | 36  | C2   | 1     |
| 26  | 38  | D2   | 2     |
| 28  | 40  | E2   | 3     |
| 29  | 41  | F2   | 4     |
| 2B  | 43  | G2   | 5     |
| 2D  | 45  | A2   | 6     |
| 2F  | 47  | B2   | 7     |
| 30  | 48  | C3   | 8     |
| 32  | 50  | D3   | 9     |
| 34  | 52  | E3   | 10    |
| 35  | 53  | F3   | 11    |
| 37  | 55  | G3   | 12    |
| 39  | 57  | A3   | 13    |
| 3B  | 59  | B3   | 14    |
| 3C  | 60  | C4   | 15    |
| 3E  | 62  | D4   | 16    |

### Note → Pattern

| Hex | Dec | Note | Pattern |
| --- | --- | ---- | ------- |
| 40  | 64  | E4   | A01     |
| 41  | 65  | F4   | A02     |
| 43  | 67  | G4   | A03     |
| 45  | 69  | A4   | A04     |
| 47  | 71  | B4   | A05     |
| 48  | 72  | C5   | A06     |
| 4A  | 74  | D5   | A07     |
| 4C  | 76  | E5   | A08     |
| 4D  | 77  | F5   | A09     |
| 4F  | 79  | G5   | A10     |
| 51  | 81  | A5   | A11     |
| 53  | 83  | B5   | A12     |
| 54  | 84  | C6   | A13     |
| 56  | 86  | D6   | A14     |
| 58  | 88  | E6   | A15     |
| 59  | 89  | F6   | A16     |

This mapping can be changed in the MIDI map editor.

## 2. CC Mappings

### MIDI Base Channel + 0 (Tracks 1–4: BD, SD, HT, MT):

| Hex | Dec | Parameter                             | Trn | Rec |
| --- | --- | ------------------------------------- | --- | --- |
| 08  | 8   | Track 1 BD - Level                    | X   | X   |
| 09  | 9   | Track 2 SD - Level                    | X   | X   |
| 0A  | 10  | Track 3 HT - Level                    | X   | X   |
| 0B  | 11  | Track 4 MT - Level                    | X   | X   |
| 0C  | 12  | Track 1 BD - Mute (>0 mutes trk)      |     | X   |
| 0D  | 13  | Track 2 SD - Mute (>0 mutes trk)      |     | X   |
| 0E  | 14  | Track 3 HT - Mute (>0 mutes trk)      |     | X   |
| 0F  | 15  | Track 4 MT - Mute (>0 mutes trk)      |     | X   |
| 10  | 16  | Track 1 BD - Machine parameter 1      | X   | X   |
| 11  | 17  | Track 1 BD - Machine parameter 2      | X   | X   |
| 12  | 18  | Track 1 BD - Machine parameter 3      | X   | X   |
| 13  | 19  | Track 1 BD - Machine parameter 4      | X   | X   |
| 14  | 20  | Track 1 BD - Machine parameter 5      | X   | X   |
| 15  | 21  | Track 1 BD - Machine parameter 6      | X   | X   |
| 16  | 22  | Track 1 BD - Machine parameter 7      | X   | X   |
| 17  | 23  | Track 1 BD - Machine parameter 8      | X   | X   |
| 18  | 24  | Track 1 BD - AM Depth                 | X   | X   |
| 19  | 25  | Track 1 BD - AM Rate                  | X   | X   |
| 1A  | 26  | Track 1 BD - EQ Freq                  | X   | X   |
| 1B  | 27  | Track 1 BD - EQ Gain                  | X   | X   |
| 1C  | 28  | Track 1 BD - Filter base frequency    | X   | X   |
| 1D  | 29  | Track 1 BD - Filter width             | X   | X   |
| 1E  | 30  | Track 1 BD - Filter Q                 | X   | X   |
| 1F  | 31  | Track 1 BD - Sample rate reduction    | X   | X   |
| 20  | 32  | Track 1 BD - Distortion               | X   | X   |
| 21  | 33  | Track 1 BD - Volume                   | X   | X   |
| 22  | 34  | Track 1 BD - Pan                      | X   | X   |
| 23  | 35  | Track 1 BD - Delay send               | X   | X   |
| 24  | 36  | Track 1 BD - Reverb send              | X   | X   |
| 25  | 37  | Track 1 BD - LFO Speed                | X   | X   |
| 26  | 38  | Track 1 BD - LFO Amount               | X   | X   |
| 27  | 39  | Track 1 BD - LFO Shape                | X   | X   |
| 28  | 40  | Track 2 SD - Machine parameter 1      | X   | X   |
| …   | …   | (same 24-parameter layout as Track 1) | X   | X   |
| 3F  | 63  | Track 2 SD - LFO Shape                | X   | X   |
| 48  | 72  | Track 3 HT - Machine parameter 1      | X   | X   |
| …   | …   | (same 24-parameter layout as Track 1) | X   | X   |
| 5F  | 95  | Track 3 HT - LFO Shape                | X   | X   |
| 60  | 96  | Track 4 MT - Machine parameter 1      | X   | X   |
| …   | …   | (same 24-parameter layout as Track 1) | X   | X   |
| 77  | 119 | Track 4 MT - LFO Shape                | X   | X   |

### MIDI Base Channel + 1 (Tracks 5–8: LT, CP, RS, CB):

| Hex | Dec | Parameter                          | Trn | Rec |
| --- | --- | ---------------------------------- | --- | --- |
| 08  | 8   | Track 5 LT - Level                 | X   | X   |
| 09  | 9   | Track 6 CP - Level                 | X   | X   |
| 0A  | 10  | Track 7 RS - Level                 | X   | X   |
| 0B  | 11  | Track 8 CB - Level                 | X   | X   |
| 0C  | 12  | Track 5 LT - Mute (>0 mutes trk)   |     | X   |
| 0D  | 13  | Track 6 CP - Mute (>0 mutes trk)   |     | X   |
| 0E  | 14  | Track 7 RS - Mute (>0 mutes trk)   |     | X   |
| 0F  | 15  | Track 8 CB - Mute (>0 mutes trk)   |     | X   |
| 10  | 16  | Track 5 LT - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)         | X   | X   |
| 27  | 39  | Track 5 LT - LFO Shape             | X   | X   |
| 28  | 40  | Track 6 CP - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)         | X   | X   |
| 3F  | 63  | Track 6 CP - LFO Shape             | X   | X   |
| 48  | 72  | Track 7 RS - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)         | X   | X   |
| 5F  | 95  | Track 7 RS - LFO Shape             | X   | X   |
| 60  | 96  | Track 8 CB - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)         | X   | X   |
| 77  | 119 | Track 8 CB - LFO Shape             | X   | X   |

### MIDI Base Channel + 2 (Tracks 9–12: CH, OH, RC, CC):

| Hex | Dec | Parameter                           | Trn | Rec |
| --- | --- | ----------------------------------- | --- | --- |
| 08  | 8   | Track 9  CH - Level                 | X   | X   |
| 09  | 9   | Track 10 OH - Level                 | X   | X   |
| 0A  | 10  | Track 11 RC - Level                 | X   | X   |
| 0B  | 11  | Track 12 CC - Level                 | X   | X   |
| 0C  | 12  | Track 9  CH - Mute (>0 mutes trk)   |     | X   |
| 0D  | 13  | Track 10 OH - Mute (>0 mutes trk)   |     | X   |
| 0E  | 14  | Track 11 RC - Mute (>0 mutes trk)   |     | X   |
| 0F  | 15  | Track 12 CC - Mute (>0 mutes trk)   |     | X   |
| 10  | 16  | Track 9  CH - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)          | X   | X   |
| 27  | 39  | Track 9  CH - LFO Shape             | X   | X   |
| 28  | 40  | Track 10 OH - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)          | X   | X   |
| 3F  | 63  | Track 10 OH - LFO Shape             | X   | X   |
| 48  | 72  | Track 11 RC - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)          | X   | X   |
| 5F  | 95  | Track 11 RC - LFO Shape             | X   | X   |
| 60  | 96  | Track 12 CC - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)          | X   | X   |
| 77  | 119 | Track 12 CC - LFO Shape             | X   | X   |

### MIDI Base Channel + 3 (Tracks 13–16: M1, M2, M3, M4):

| Hex | Dec | Parameter                           | Trn | Rec |
| --- | --- | ----------------------------------- | --- | --- |
| 08  | 8   | Track 13 M1 - Level                 | X   | X   |
| 09  | 9   | Track 14 M2 - Level                 | X   | X   |
| 0A  | 10  | Track 15 M3 - Level                 | X   | X   |
| 0B  | 11  | Track 16 M4 - Level                 | X   | X   |
| 0C  | 12  | Track 13 M1 - Mute (>0 mutes trk)   |     | X   |
| 0D  | 13  | Track 14 M2 - Mute (>0 mutes trk)   |     | X   |
| 0E  | 14  | Track 15 M3 - Mute (>0 mutes trk)   |     | X   |
| 0F  | 15  | Track 16 M4 - Mute (>0 mutes trk)   |     | X   |
| 10  | 16  | Track 13 M1 - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)          | X   | X   |
| 27  | 39  | Track 13 M1 - LFO Shape             | X   | X   |
| 28  | 40  | Track 14 M2 - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)          | X   | X   |
| 3F  | 63  | Track 14 M2 - LFO Shape             | X   | X   |
| 48  | 72  | Track 15 M3 - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)          | X   | X   |
| 5F  | 95  | Track 15 M3 - LFO Shape             | X   | X   |
| 60  | 96  | Track 16 M4 - Machine parameter 1   | X   | X   |
| …   | …   | (same 24-parameter layout)          | X   | X   |
| 77  | 119 | Track 16 M4 - LFO Shape             | X   | X   |
