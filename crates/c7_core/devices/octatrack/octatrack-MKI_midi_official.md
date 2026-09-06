# Octatrack MKI MIDI implementation

`Octatrack-User-Manual_ENG-OS1.40A_220204.pdf`: Pages 137 - 142

---

All parameters found in the TRACK PARAMETER MAIN pages are controllable by MIDI control change messages. It is also possible to trig tracks, samples, slices, slots, and track recorders as well as to play samples chromatically from an external MIDI device.

Which tracks responding to which MIDI channels are configured in the PROJECT > MIDI > CHANNELS menu.

The MIDI note mapping varies depending on what TRIG mode and MIDI MAP configuration you chose. For more information, please see "12.7 TRIG MODES" on page 69. "12.8 MIDI NOTE MAPPING FOR AUDIO TRACKS" on page 71. Also the AUDIO NOTE IN parameter's setting affects the MIDI note mapping. "8.7.1 CONTROL" on page 39.

## 1. Standard Note Mapping

In this mode, incoming MIDI notes trigger several functions.

This MIDI note mapping is the default setting and is used when choosing the TRACKS trig mode (and set the **AUDIO NOTE IN** parameter to FOLLOW FW) and in the STANDARD MIDI MAP configuration. This mapping is also used then the AUDIO NOTE IN parameter is set to STANDARD. For more information, please see "12.7.1 TRACKS" on page 69. "12.8 MIDI NOTE MAPPING FOR AUDIO TRACKS" on page 71. "8.7.1 CONTROL" on page 39.

The Octatrack MKI responds to MIDI notes as follows:

| Note     | Function                     |
| -------- | ---------------------------- |
| C1 (24)  | Audio Track 1 Play           |
| C#1 (25) | Audio Track 2 Play           |
| D1 (26)  | Audio Track 3 Play           |
| D#1 (27) | Audio Track 4 Play           |
| E1 (28)  | Audio Track 5 Play           |
| F1 (29)  | Audio Track 6 Play           |
| F#1 (30) | Audio Track 7 Play           |
| G1 (31)  | Audio Track 8 Play           |
| G#1 (32) |                              |
| A1 (33)  | Sequencer Stop               |
| A#1 (34) | Sequencer Start              |
| B1 (35)  | Sequencer Restart            |
| C2 (36)  | Audio Track 1 Sample Trigger |
| C#2 (37) | Audio Track 2 Sample Trigger |
| D2 (38)  | Audio Track 3 Sample Trigger |
| D#2 (39) | Audio Track 4 Sample Trigger |
| E2 (40)  | Audio Track 5 Sample Trigger |
| F2 (41)  | Audio Track 6 Sample Trigger |
| F#2 (42) | Audio Track 7 Sample Trigger |
| G2 (43)  | Audio Track 8 Sample Trigger |
| G#2 (44) |                              |
| A2 (45)  |                              |
| A#2 (46) |                              |
| B2 (47)  |                              |
| C3 (48)  | MIDI Track 1 Play            |
| C#3 (49) | MIDI Track 2 Play            |
| D3 (50)  | MIDI Track 3 Play            |
| D#3 (51) | MIDI Track 4 Play            |
| E3 (52)  | MIDI Track 5 Play            |
| F3 (53)  | MIDI Track 6 Play            |
| F#3 (54) | MIDI Track 7 Play            |
| G3 (55)  | MIDI Track 8 Play            |
| G#3 (56) |                              |
| A3 (57)  |                              |
| A#3 (58) |                              |
| B3 (59)  |                              |
| C4 (60)  | Combo rec                    |
| C#4 (61) | INAB rec                     |
| D4 (62)  | INCD rec                     |
| D#4 (63) | SRC3 rec                     |
| E4 (64)  | Pickup Play/Stop             |
| F4 (65)  | Multiply                     |
| F#4 (66) | Mute active track            |
| G4 (67)  | Cue active track             |
| G#4 (68) | Active track up              |
| A4 (69)  | Active track down            |
| A#4 (70) | Toggle Seq sync              |
| B4 (71)  | Toggle Seq sync + start      |
| C5 (72)  | Track Sample Pitch -12       |
| C#5 (73) | Track Sample Pitch -11       |
| D5 (74)  | Track Sample Pitch -10       |
| D#5 (75) | Track Sample Pitch -9        |
| E5 (76)  | Track Sample Pitch -8        |
| F5 (77)  | Track Sample Pitch -7        |
| F#5 (78) | Track Sample Pitch -6        |
| G5 (79)  | Track Sample Pitch -5        |
| G#5 (80) | Track Sample Pitch -4        |
| A5 (81)  | Track Sample Pitch -3        |
| A#5 (82) | Track Sample Pitch -2        |
| B5 (83)  | Track Sample Pitch -1        |
| C6 (84)  | Track Sample Pitch 0         |
| C#6 (85) | Track Sample Pitch +1        |
| D6 (86)  | Track Sample Pitch +2        |
| D#6 (87) | Track Sample Pitch +3        |
| E6 (88)  | Track Sample Pitch +4        |
| F6 (89)  | Track Sample Pitch +5        |
| F#6 (90) | Track Sample Pitch +6        |
| G6 (91)  | Track Sample Pitch +7        |
| G#6 (92) | Track Sample Pitch +8        |
| A6 (93)  | Track Sample Pitch +9        |
| A#6 (94) | Track Sample Pitch +10       |
| B6 (95)  | Track Sample Pitch +11       |
| C7 (96)  | Track Sample Pitch +12       |

> [!NOTE]
> • Chromatic trig keys sent to the AUTO channel or the MIDI channel of the active track can, when in LIVE RECORDING mode or holding a note trig, be used to enter note trigs with locked PTCH parameters.
>
> • CC messages sent to the auto channel or the MIDI channel of the active track can, when in LIVE RECORDING mode or when entering parameter and scene locks, be recorded.
>
> • Active track up and down only works on the auto channel.

## 2. Chromatic Note Mapping

In this mode, incoming MIDI notes trigger the track's sample chromatically.

This MIDI note mapping is used when choosing the CHROMATIC TRIG mode (and set the AUDIO NOTE IN parameter to FOLLOW FW) and in the CHROMATIC MIDI MAP configuration. For more information, please see "12.7.2 CHROMATIC" on page 70. "12.8 MIDI NOTE MAPPING FOR AUDIO TRACKS" on page 71. "8.7.1 CONTROL" on page 39.

The Octatrack MKI responds to MIDI Notes as follows:

| NOTE              | FUNCTION                                        |
| ----------------- | ----------------------------------------------- |
| C5 (72) – C7 (96) | Track Sample Pitch -12 – Track Sample Pitch +12 |

> [!NOTE]
>• The 2-octave range is only valid for audio tracks. On MIDI tracks, notes can be played and recorded over the full 128 MIDI note range.

## 3. Slots Note Mapping

In this mode, incoming MIDI notes trigger the samples in the project's Flex and Static sample slot lists. For more information, please see "8.3 LOADING SAMPLES TO THE SAMPLE SLOTS" on page 29.

This MIDI note mapping is used when choosing the SLOT TRIG mode (and set the AUDIO NOTE IN parameter to FOLLOW FW) and in the SLOT MIDI MAP configuration. For more information, please see "12.7.3 SLOTS" on page 70. "12.8 MIDI NOTE MAPPING FOR AUDIO TRACKS" on page 71. "8.7.1 CONTROL" on page 39.

The Octatrack MKI responds to MIDI Notes as follows:

| Note               | Function          |
| ------------------ | ----------------- |
| C-1 (0) – G9 (127) | Slot 1 – Slot 128 |

> [!NOTE]
> • The sample in the first slot is played on C-1, the next on C#-1 and so on, expanding upward depending on how many slots are filled with samples.

## 4. Slices Note Mapping

In this mode, incoming MIDI notes triggers the tracks sample slices individually.

This MIDI note mapping is used when choosing the SLICES TRIG mode (and set the AUDIO NOTE IN parameter to FOLLOW FW) and in the SLICES MIDI MAP configuration. For more information, please see "12.7.4 SLICES" on page 70. "12.8 MIDI NOTE MAPPING FOR AUDIO TRACKS" on page 71. "8.7.1 CONTROL" on page 39.

The Octatrack MKI responds to MIDI Notes as follows:

| Note               | Function           |
| ------------------ | ------------------ |
| C0 (12) – D#5 (75) | Slice 1 – Slice 64 |

> [!NOTE]
> • The first slice is played on C0, the next on C#0 and so on, expanding upward depending on how many slices the sample has.

> [!WARNING]
> The tracks sample must contain slices for this mode to work as intended. For more information, please see "13.2.2 SLICE" on page 82.

## 5. Quick Mute Note Mapping

In this mode, incoming MIDI notes toggles mute on and off for the audio and MIDI tracks.

This MIDI note mapping is used when choosing the QUICK MUTE TRIG mode (and set the AUDIO NOTE IN parameter to FOLLOW FW) and in the QUICK MUTE MIDI MAP configuration. For more information, please see "12.7.5 QUICK MUTE" on page 71. "12.8 MIDI NOTE MAPPING FOR AUDIO TRACKS" on page 71. "8.7.1 CONTROL" on page 39.

The Octatrack MKI responds to MIDI Notes as follows:

| Note                | Function                        |
| ------------------- | ------------------------------- |
| C0 (12) – G0 (19)   | Mutes/unmutes audio track 1 – 8 |
| G#0 (20) – D#1 (27) | Mutes/unmutes MIDI track 1 – 8  |

## 6. Delay Control Note Mapping

In this mode, incoming MIDI notes controls the behavior of the track's delay settings. (For tracks that uses
the Echo Freeze Delay effect.)

This MIDI note mapping is used when choosing the DELAY CONTROL TRIG mode (and set the **AUDIO NOTE IN** parameter to FOLLOW FW) and in the DELAY CONTROL MIDI MAP configuration. For more information, please see "12.7.6 DELAY CONTROL" on page 71. "12.8 MIDI NOTE MAPPING FOR AUDIO TRACKS" on page 71. "8.7.1 CONTROL" on page 39.

The Octatrack MKI responds to MIDI Notes as follows:

| Note                | Function                                                     |
| ------------------- | ------------------------------------------------------------ |
| C0 (12) – G0 (19)   | Selects delay TIME values of 1, 2, 4, 8, 16, 32 ,64, and 128 |
| G#0 (20) – D#1 (27) | Selects the track to control (1 – 8)                         |


| Note                | Function                                                     |
| ------------------- | ------------------------------------------------------------ |
| C2 (36) – G2 (43)   | Selects delay TIME values of 1, 3, 6, 12, 24, 48, 96 and 128 |
| G#2 (44) – D#3 (51) | Selects the track to control (1 – 8)                         |

> [!NOTE]
> The track select and delay time select notes are used in conjunction with each other to change the setting. The note that selects the track must be held (note on) while sending the second note that selects the TIME value.

## 7. Control Change Mapping

| Hex   | Dec | Ctrl Parameter                           | TRN | REC |
| ----- | --- | ---------------------------------------- | :-: | :-: |
| `$07` | 07  | Track level                              |     | X   |
| `$08` | 08  | Track balance                            |     | X   |
| `$10` | 16  | Playback param #1                        | X   | X   |
| `$11` | 17  | Playback param #2                        | X   | X   |
| `$12` | 18  | Playback param #3                        | X   | X   |
| `$13` | 19  | Playback param #4                        | X   | X   |
| `$14` | 20  | Playback param #5                        | X   | X   |
| `$15` | 21  | Playback param #6                        | X   | X   |
| `$16` | 22  | Amp param #1 (Attack)                    | X   | X   |
| `$17` | 23  | Amp param #2 (Hold)                      | X   | X   |
| `$18` | 24  | Amp param #3 (Release)                   | X   | X   |
| `$19` | 25  | Amp param #4 (Volume)                    | X   | X   |
| `$1A` | 26  | Amp param #5 (Balance)                   | X   | X   |
| `$1B` | 27  | Amp param #6 (N/A)                       | X   | X   |
| `$1C` | 28  | LFO param #1 (Speed 1)                   | X   | X   |
| `$1D` | 29  | LFO param #2 (Speed 2)                   | X   | X   |
| `$1E` | 30  | LFO param #3 (Speed 3)                   | X   | X   |
| `$1F` | 31  | LFO param #4 (Depth 1)                   | X   | X   |
| `$20` | 32  | LFO param #5 (Depth 2)                   | X   | X   |
| `$21` | 33  | LFO param #6 (Depth 3)                   | X   | X   |
| `$22` | 34  | FX1 param #1                             | X   | X   |
| `$23` | 35  | FX1 param #2                             | X   | X   |
| `$24` | 36  | FX1 param #3                             | X   | X   |
| `$25` | 37  | FX1 param #4                             | X   | X   |
| `$26` | 38  | FX1 param #5                             | X   | X   |
| `$27` | 39  | FX1 param #6                             | X   | X   |
| `$28` | 40  | FX2 param #1                             | X   | X   |
| `$29` | 41  | FX2 param #2                             | X   | X   |
| `$2A` | 42  | FX2 param #3                             | X   | X   |
| `$2B` | 43  | FX2 param #4                             | X   | X   |
| `$2C` | 44  | FX2 param #5                             | X   | X   |
| `$2D` | 45  | FX2 param #6                             | X   | X   |
| `$2E` | 46  | Track level                              | X   | X   |
| `$2F` | 47  | Cue level                                | X   | X   |
| `$30` | 48  | Crossfader                               | X   | X   |
| `$31` | 49  | Track Mute [0]=Unmuted, [1-127]=Muted    | X   | X   |
| `$32` | 50  | Track Solo [0]=No solo, [1-127]=Soloed   | X   | X   |
| `$33` | 51  | Track Cue [0]=Not Cued, [1-127]=Cued     | X   | X   |
| `$34` | 52  | Track Arm [0]=Trk disarm, [1-127]=Arm    | X   | X   |
| `$35` | 53  | Recrdr Arm [0]=Rec disarm, [1-127]=Arm   | X   | X   |
| `$36` | 54  | All Arm [0]=All disarm, [1-127]=Arm      | X   | X   |
| `$37` | 55  | Scene A Select                           | X   | X   |
| `$38` | 56  | Scene B Select                           | X   | X   |
| `$39` | 57  | Pickup SRC AB level                      |     | X   |
| `$3A` | 58  | Pickup SRC CD level                      |     | X   |
| `$3B` | 59  | Note on [0]=C-1, 127=G9)                 |     | X   |
| `$3C` | 60  | Note off [0] =C-1, 127=G9)               |     | X   |
| `$3D` | 61  | Send request, 0 = all audio CC           |     |     |
| `$70` | 112 | MIDI Trk 1 Mute [0]=Unmute, [1-127]=Mute | X   | X   |
| `$71` | 113 | MIDI Trk 2 Mute [0]=Unmute, [1-127]=Mute | X   | X   |
| `$72` | 114 | MIDI Trk 3 Mute [0]=Unmute, [1-127]=Mute | X   | X   |
| `$73` | 115 | MIDI Trk 4 Mute [0]=Unmute, [1-127]=Mute | X   | X   |
| `$74` | 116 | MIDI Trk 5 Mute [0]=Unmute, [1-127]=Mute | X   | X   |
| `$75` | 117 | MIDI Trk 6 Mute [0]=Unmute, [1-127]=Mute | X   | X   |
| `$76` | 118 | MIDI Trk 7 Mute [0]=Unmute, [1-127]=Mute | X   | X   |
| `$77` | 119 | MIDI Trk 8 Mute [0]=Unmute, [1-127]=Mute | X   | X   |
| `$78` | 120 | MIDI Trk 1 Solo [0]=Unsolo, [1-127]=Solo | X   | X   |
| `$79` | 121 | MIDI Trk 2 Solo [0]=Unsolo, [1-127]=Solo | X   | X   |
| `$7A` | 122 | MIDI Trk 3 Solo [0]=Unsolo, [1-127]=Solo | X   | X   |
| `$7B` | 123 | MIDI Trk 4 Solo [0]=Unsolo, [1-127]=Solo | X   | X   |
| `$7C` | 124 | MIDI Trk 5 Solo [0]=Unsolo, [1-127]=Solo | X   | X   |
| `$7D` | 125 | MIDI Trk 6 Solo [0]=Unsolo, [1-127]=Solo | X   | X   |
| `$7E` | 126 | MIDI Trk 7 Solo [0]=Unsolo, [1-127]=Solo | X   | X   |
| `$7F` | 127 | MIDI Trk 8 Solo [0]=Unsolo, [1-127]=Solo | X   | X   |

## 8. MIDI Mode Control Change Mapping

While in MIDI mode and CC DIRECT CONNECT is deactivated, the auto channel responds to the following CC messages.

| Hex   | Dec | Ctrl Parameter                              | TRN | REC |
| ----- | --- | ------------------------------------------- | :-: | :-: |
| `$10` | 16  | Playback param #1 (Note)                    |     | X   |
| `$11` | 17  | Playback param #2 (Velocity)                |     | X   |
| `$12` | 18  | Playback param #3 (Length)                  |     | X   |
| `$13` | 19  | Playback param #4 (Note 2)                  |     | X   |
| `$14` | 20  | Playback param #5 (Note 3)                  |     | X   |
| `$15` | 21  | Playback param #6 (Note 4)                  |     | X   |
| `$16` | 22  | Amp param #1 (Transpose)                    |     | X   |
| `$17` | 23  | Amp param #2 (Legato)                       |     | X   |
| `$18` | 24  | Amp param #3 (Mode)                         |     | X   |
| `$19` | 25  | Amp param #4 (Speed)                        |     | X   |
| `$1A` | 26  | Amp param #5 (Octave Range)                 |     | X   |
| `$1B` | 27  | Amp param #6 (Arp Note Length)              |     | X   |
| `$1C` | 28  | MIDI LFO param #1 (Speed 1)                 |     | X   |
| `$1D` | 29  | MIDI LFO param #2 (Speed 2)                 |     | X   |
| `$1E` | 30  | MIDI LFO param #3 (Speed 3)                 |     | X   |
| `$1F` | 31  | MIDI LFO param #4 (Depth 1)                 |     | X   |
| `$20` | 32  | MIDI LFO param #5 (Depth 2)                 |     | X   |
| `$21` | 33  | MIDI LFO param #6 (Depth 3)                 |     | X   |
| `$22` | 34  | Pitch bend                                  |     | X   |
| `$23` | 35  | Aftertouch                                  |     | X   |
| `$24` | 36  | CC #1 param                                 |     | X   |
| `$25` | 37  | CC #2 param                                 |     | X   |
| `$26` | 38  | CC #3 param                                 |     | X   |
| `$27` | 39  | CC #4 param                                 |     | X   |
| `$28` | 40  | CC #5 param                                 |     | X   |
| `$29` | 41  | CC #6 param                                 |     | X   |
| `$2A` | 42  | CC #7 param                                 |     | X   |
| `$2B` | 43  | CC #8 param                                 |     | X   |
| `$2C` | 44  | CC #9 param                                 |     | X   |
| `$2D` | 45  | CC #10 param                                |     | X   |
| `$31` | 49  | MIDI Track Mute [0]=Unmuted, [1-127]=Muted  |     | X   |
| `$32` | 50  | MIDI Track Solo [0]=No solo, [1-127]=Soloed |     | X   |
| `$36` | 54  | All Arm [0]=All disarm, [1-127]=Arm         |     | X   |
| `$37` | 55  | Scene A Select                              |     | X   |
| `$38` | 56  | Scene B Select                              |     | X   |
| `$70` | 112 | MIDI Trk 1 Mute [0]=Unmute, [1-127]=Mute    |     | X   |
| `$71` | 113 | MIDI Trk 2 Mute [0]=Unmute, [1-127]=Mute    |     | X   |
| `$72` | 114 | MIDI Trk 3 Mute [0]=Unmute, [1-127]=Mute    |     | X   |
| `$73` | 115 | MIDI Trk 4 Mute [0]=Unmute, [1-127]=Mute    |     | X   |
| `$74` | 116 | MIDI Trk 5 Mute [0]=Unmute, [1-127]=Mute    |     | X   |
| `$75` | 117 | MIDI Trk 6 Mute [0]=Unmute, [1-127]=Mute    |     | X   |
| `$76` | 118 | MIDI Trk 7 Mute [0]=Unmute, [1-127]=Mute    |     | X   |
| `$77` | 119 | MIDI Trk 8 Mute [0]=Unmute, [1-127]=Mute    |     | X   |
| `$78` | 120 | MIDI Trk 1 Solo [0]=Unsolo, [1-127]=Solo    |     | X   |
| `$79` | 121 | MIDI Trk 2 Solo [0]=Unsolo, [1-127]=Solo    |     | X   |
| `$7A` | 122 | MIDI Trk 3 Solo [0]=Unsolo, [1-127]=Solo    |     | X   |
| `$7B` | 123 | MIDI Trk 4 Solo [0]=Unsolo, [1-127]=Solo    |     | X   |
| `$7C` | 124 | MIDI Trk 5 Solo [0]=Unsolo, [1-127]=Solo    |     | X   |
| `$7D` | 125 | MIDI Trk 6 Solo [0]=Unsolo, [1-127]=Solo    |     | X   |
| `$7E` | 126 | MIDI Trk 7 Solo [0]=Unsolo, [1-127]=Solo    |     | X   |
| `$7F` | 127 | MIDI Trk 8 Solo [0]=Unsolo, [1-127]=Solo    |     | X   |
