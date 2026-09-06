# Monomachine MIDI implementation

`monomachine_manual_OS1.32.pdf`: Pages 141 - 146

---

All Data Entry parameters of the Monomachine are accessible by both MIDI control change messages and NRPN MIDI messages.

The Monomachine uses up to 6 MIDI channels starting from the Base Channel for the internal sequencer and to control the six tracks of sound generation. This is set in `Global slot → Control → Midichanls`.

In addition, one or none channel can be allocated to each of Multi Trig, Multi Map, and the Auto Channel.

The MIDI sequencer can additionally use 6 MIDI channels set in `Global slot → Midi seq → Midiseq set`.

When receiving MIDI, the MIDI sequencer channels are not reachable if overlapped by any other channel assignment. Overlapping is possible when sending MIDI.

By setting the span to 0 and turning Multi Trig, Multi Map, and the Auto Channel off, no MIDI is sent from the Monomachine except if the MIDI sequencer is used.

**Legend:**

| Symbol | Meaning                                     |
| ------ | ------------------------------------------- |
| X      | Acknowledged                                |
| K      | Only sent from the Keyboard version (SFX-6) |
| B      | Only received on the MIDI Base Channel      |

## 1. Note On & Note Off Messages

### Track Channels

| Note | MIDI Channel    | Track | Action                 | Trn | Rec |
| ---- | --------------- | ----- | ---------------------- | --- | --- |
| ON   | Basechannel + 0 | 1     | Trig amp, filter & lfo | X   | X   |
| OFF  | Basechannel + 0 | 1     | Amp in release phase   | X   | X   |
| ON   | Basechannel + 1 | 2     | Trig amp, filter & lfo | X   | X   |
| OFF  | Basechannel + 1 | 2     | Amp in release phase   | X   | X   |
| ON   | Basechannel + 2 | 3     | Trig amp, filter & lfo | X   | X   |
| OFF  | Basechannel + 2 | 3     | Amp in release phase   | X   | X   |
| ON   | Basechannel + 3 | 4     | Trig amp, filter & lfo | X   | X   |
| OFF  | Basechannel + 3 | 4     | Amp in release phase   | X   | X   |
| ON   | Basechannel + 4 | 5     | Trig amp, filter & lfo | X   | X   |
| OFF  | Basechannel + 4 | 5     | Amp in release phase   | X   | X   |
| ON   | Basechannel + 5 | 6     | Trig amp, filter & lfo | X   | X   |
| OFF  | Basechannel + 5 | 6     | Amp in release phase   | X   | X   |

### Multi Trig, Multi Map & Auto Channel

| Note | MIDI Channel | Mode       | Action                                                                                                    | Trn | Rec |
| ---- | ------------ | ---------- | --------------------------------------------------------------------------------------------------------- | --- | --- |
| ON   | Multi Trig   | All Track  | Distributed to track 1–6                                                                                  | X   | X   |
| OFF  | Multi Trig   | All Track  | Distributed to track 1–6                                                                                  | X   | X   |
| ON   | Multi Trig   | Split Key  | Distributed according to split setting                                                                    | X   | X   |
| OFF  | Multi Trig   | Split Key  | Distributed according to split setting                                                                    | X   | X   |
| ON   | Multi Trig   | Seq Start  | Pattern player start or if in play restart. Multi ENV in attack phase. Pattern transpose relative MIDI-C4 | X   | X   |
| OFF  | Multi Trig   | Seq Start  | Multi ENV in release phase                                                                                | X   | X   |
| ON   | Multi Trig   | Seq Transp | Pattern player start only if in stop. Multi ENV in attack phase. Pattern transpose relative MIDI-C4       | X   | X   |
| OFF  | Multi Trig   | Seq Transp | Multi ENV in release phase                                                                                | X   | X   |
| ON   | Multi Map    |            | Action based on settings in global-multimap                                                               | X   | X   |
| OFF  | Multi Map    |            | Action based on settings in global-multimap                                                               | X   | X   |
| ON   | Auto Channel |            | Distributed to active track                                                                               |     | X   |
| OFF  | Auto Channel |            | Distributed to active track                                                                               |     | X   |

## 2. Control Change Messages

### MIDI Base Channel + 0

| Hex | Dec | Parameter                               | Trn | Rec |
| --- | --- | --------------------------------------- | --- | --- |
| 01  | 1   | Track 1 - Joystick Up                   | K   | X   |
| 02  | 2   | Track 1 - Joystick Down                 | K   | X   |
| 03  | 3   | Track 1 - Mute (0=Unmuted, 1–127=Muted) |     | X   |
| 06  | 6   | NRPN Parameter Val                      |     | B   |
| 07  | 7   | Track 1 - Level                         | X   | X   |
| 0A  | 10  | Track 1 - Amp Pan                       |     | X   |
| 30  | 48  | Track 1 - Synthesis parameter 1         | X   | X   |
| 31  | 49  | Track 1 - Synthesis parameter 2         | X   | X   |
| 32  | 50  | Track 1 - Synthesis parameter 3         | X   | X   |
| 33  | 51  | Track 1 - Synthesis parameter 4         | X   | X   |
| 34  | 52  | Track 1 - Synthesis parameter 5         | X   | X   |
| 35  | 53  | Track 1 - Synthesis parameter 6         | X   | X   |
| 36  | 54  | Track 1 - Synthesis parameter 7         | X   | X   |
| 37  | 55  | Track 1 - Synthesis parameter 8         | X   | X   |
| 38  | 56  | Track 1 - Amp Attack                    | X   | X   |
| 39  | 57  | Track 1 - Amp Hold                      | X   | X   |
| 3A  | 58  | Track 1 - Amp Decay                     | X   | X   |
| 3B  | 59  | Track 1 - Amp Release                   | X   | X   |
| 3C  | 60  | Track 1 - Amp Dist                      | X   | X   |
| 3D  | 61  | Track 1 - Amp Vol                       | X   | X   |
| 3E  | 62  | Track 1 - Amp Pan                       | X   | X   |
| 3F  | 63  | Track 1 - Amp Portamento                | X   | X   |
| 48  | 72  | Track 1 - Filter Base                   | X   | X   |
| 49  | 73  | Track 1 - Filter Width                  | X   | X   |
| 4A  | 74  | Track 1 - Filter HPQ                    | X   | X   |
| 4B  | 75  | Track 1 - Filter LPQ                    | X   | X   |
| 4C  | 76  | Track 1 - Filter Attack                 | X   | X   |
| 4D  | 77  | Track 1 - Filter Decay                  | X   | X   |
| 4E  | 78  | Track 1 - Filter Base Offset            | X   | X   |
| 4F  | 79  | Track 1 - Filter Width Offset           | X   | X   |
| 50  | 80  | Track 1 - Effects EQ Freq               | X   | X   |
| 51  | 81  | Track 1 - Effects EQ Gain               | X   | X   |
| 52  | 82  | Track 1 - Effects SRR                   | X   | X   |
| 53  | 83  | Track 1 - Effects Delay Time            | X   | X   |
| 54  | 84  | Track 1 - Effects Delay Send            | X   | X   |
| 55  | 85  | Track 1 - Effects Delay Feedback        | X   | X   |
| 56  | 86  | Track 1 - Effects Delay Filter Base     | X   | X   |
| 57  | 87  | Track 1 - Effects Delay Filter Width    | X   | X   |
| 58  | 88  | Track 1 - LFO 1 Page                    | X   | X   |
| 59  | 89  | Track 1 - LFO 1 Dest                    | X   | X   |
| 5A  | 90  | Track 1 - LFO 1 Trig                    | X   | X   |
| 5B  | 91  | Track 1 - LFO 1 Wave                    | X   | X   |
| 5C  | 92  | Track 1 - LFO 1 Multiplier              | X   | X   |
| 5D  | 93  | Track 1 - LFO 1 Speed                   | X   | X   |
| 5E  | 94  | Track 1 - LFO 1 Interlace               | X   | X   |
| 5F  | 95  | Track 1 - LFO 1 Depth                   | X   | X   |
| 62  | 98  | NRPN Parameter Lo (Parameter)           |     | B   |
| 63  | 99  | NRPN Parameter Hi (Track num)           |     | B   |
| 68  | 104 | Track 1 - LFO 2 Page                    | X   | X   |
| 69  | 105 | Track 1 - LFO 2 Dest                    | X   | X   |
| 6A  | 106 | Track 1 - LFO 2 Trig                    | X   | X   |
| 6B  | 107 | Track 1 - LFO 2 Wave                    | X   | X   |
| 6C  | 108 | Track 1 - LFO 2 Multiplier              | X   | X   |
| 6D  | 109 | Track 1 - LFO 2 Speed                   | X   | X   |
| 6E  | 110 | Track 1 - LFO 2 Interlace               | X   | X   |
| 6F  | 111 | Track 1 - LFO 2 Depth                   | X   | X   |
| 70  | 112 | Track 1 - LFO 3 Page                    | X   | X   |
| 71  | 113 | Track 1 - LFO 3 Dest                    | X   | X   |
| 72  | 114 | Track 1 - LFO 3 Trig                    | X   | X   |
| 73  | 115 | Track 1 - LFO 3 Wave                    | X   | X   |
| 74  | 116 | Track 1 - LFO 3 Multiplier              | X   | X   |
| 75  | 117 | Track 1 - LFO 3 Speed                   | X   | X   |
| 76  | 118 | Track 1 - LFO 3 Interlace               | X   | X   |
| 77  | 119 | Track 1 - LFO 3 Depth                   | X   | X   |
| 7B  | 123 | Track 1 - All Notes Off                 | X   | X   |

### MIDI Base Channel + 1 through + 5

Channels +1 through +5 use the identical CC map as Base Channel + 0, substituting the track number:

| Channel Offset | Track |
| -------------- | ----- |
| + 1            | 2     |
| + 2            | 3     |
| + 3            | 4     |
| + 4            | 5     |
| + 5            | 6     |

### MIDI Multi Trig Channel

| Hex | Dec | Parameter                          | Trn | Rec |
| --- | --- | ---------------------------------- | --- | --- |
| 01  | 1   | ModWheelUp - Resent to track 1–6   | K   | X   |
| 02  | 2   | ModWheelDown - Resent to track 1–6 | K   | X   |

### MIDI Multi Map Channel

| Hex | Dec | Parameter                          | Trn | Rec |
| --- | --- | ---------------------------------- | --- | --- |
| 01  | 1   | ModWheelUp - Resent to track 1–6   | K   | X   |
| 02  | 2   | ModWheelDown - Resent to track 1–6 | K   | X   |

### MIDI Auto Track Channel

All data resent to active track. Receive only.

## 3. NRPN Mapping

Received on MIDI Base Channel + 0.

### NRPN Track Address Table

| Hi (hex) | Hi (dec) | Lo  | Val      | Parameter                                               | Trn | Rec |
| -------- | -------- | --- | -------- | ------------------------------------------------------- | --- | --- |
| 00       | 0        | YY  | ZZ       | Track 1 CTRL-parameter YY to val ZZ                     |     | X   |
| 01       | 1        | YY  | ZZ       | Track 2 CTRL-parameter YY to val ZZ                     |     | X   |
| 02       | 2        | YY  | ZZ       | Track 3 CTRL-parameter YY to val ZZ                     |     | X   |
| 03       | 3        | YY  | ZZ       | Track 4 CTRL-parameter YY to val ZZ                     |     | X   |
| 04       | 4        | YY  | ZZ       | Track 5 CTRL-parameter YY to val ZZ                     |     | X   |
| 05       | 5        | YY  | ZZ       | Track 6 CTRL-parameter YY to val ZZ                     |     | X   |
| 70       | 112      |     | %XXXXXXX | Pitch XXXXXXX for Track 1                               | X   | X   |
| 71       | 113      |     | %XXXXXXX | Pitch XXXXXXX for Track 2                               | X   | X   |
| 72       | 114      |     | %XXXXXXX | Pitch XXXXXXX for Track 3                               | X   | X   |
| 73       | 115      |     | %XXXXXXX | Pitch XXXXXXX for Track 4                               | X   | X   |
| 74       | 116      |     | %XXXXXXX | Pitch XXXXXXX for Track 5                               | X   | X   |
| 75       | 117      |     | %XXXXXXX | Pitch XXXXXXX for Track 6                               | X   | X   |
| 7E       | 126      |     | %XXXXXXX | Amp in release phase on Track X (= Note Off on Track X) | X   | X   |
| 7F       | 127      |     | %XXXXALF | Trig Track X with A=Amp, L=LFO, F=Filter                | X   | X   |

### NRPN CTRL-parameter YY Mapping

Ranges are in hexadecimal.

| Range (hex) | Range (dec) | Destination            |
| ----------- | ----------- | ---------------------- |
| 00 – 1F     | 0 – 31      | Synth, Amp, Filter, Effect |
| 20 – 37     | 32 – 55     | LFO 1, LFO 2, LFO      |
| 38 – 3F     | 56 – 63     | MIDI sequencer         |
| 40 – 45     | 64 – 69     | Multi ENV              |
| 7F          | 127         | Level                  |

## 4. Other MIDI Messages

### Pitch Bend

| Channel            | Parameter                        | Trn | Rec |
| ------------------ | -------------------------------- | --- | --- |
| Base channel + 0   | Track 1 - Pitchbend              | K   | X   |
| Base channel + 1   | Track 2 - Pitchbend              | K   | X   |
| Base channel + 2   | Track 3 - Pitchbend              | K   | X   |
| Base channel + 3   | Track 4 - Pitchbend              | K   | X   |
| Base channel + 4   | Track 5 - Pitchbend              | K   | X   |
| Base channel + 5   | Track 6 - Pitchbend              | K   | X   |
| Multi Trig channel | Pitchbend - Resent to track 1–6  | K   | X   |
| Multi Map channel  | Pitchbend - Resent to track 1–6  | K   | X   |
| Auto Track channel | Pitchbend resent to active track |     | X   |

### Program Change

Customizable in the global slot.

| Value | Action            | Trn | Rec |
| ----- | ----------------- | --- | --- |
| XX    | Change pattern XX | B   | X   |

### System Common

Customizable in the global slot.

| Hex | Dec | Message               | Trn | Rec |
| --- | --- | --------------------- | --- | --- |
| F2  | 242 | Song Pointer Position | X   | X   |

### System Realtime

Customizable in the global slot.

| Hex | Dec | Message      | Trn | Rec |
| --- | --- | ------------ | --- | --- |
| F8  | 248 | Timing Clock | X   | X   |
| FA  | 250 | Start        | X   | X   |
| FB  | 251 | Continue     | X   | X   |
| FC  | 252 | Stop         | X   | X   |
