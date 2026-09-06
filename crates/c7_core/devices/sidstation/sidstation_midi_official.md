# SidStation MIDI implementation

`Elektron_SID_Users_Manual_r22b_OS1.1.pdf`: Pages 38 - 39

---

## MIDI Controller Assignments

| CC  | Name                      | Transmit | Receive | Type      |
| --- | ------------------------- | -------: | ------: | --------- |
| 1   | Modulation Wheel          |          | ✓       | CONT      |
| 16  | Direct Controller 1       | ✓        | ✓       | CONT/BOOL |
| 17  | Direct Controller 2       | ✓        | ✓       | CONT/BOOL |
| 18  | Direct Controller 3       | ✓        | ✓       | CONT/BOOL |
| 19  | Direct Controller 4       | ✓        | ✓       | CONT/BOOL |
| 20  | LFO CTRL1                 |          | ✓       | CONT      |
| 21  | LFO CTRL2                 |          | ✓       | CONT      |
| 22  | LFO CTRL3                 |          | ✓       | CONT      |
| 23  | LFO CTRL4                 |          | ✓       | CONT      |
| 24  | Oscillator 1 Active       |          | ✓       | BOOL      |
| 25  | Oscillator 2 Active       |          | ✓       | BOOL      |
| 26  | Oscillator 3 Active       |          | ✓       | BOOL      |
| 27  | Filter Cutoff             |          | ✓       | CONT      |
| 28  | Filter Envelope Depth     |          | ✓       | CONT      |
| 29  | Filter LFO Depth          |          | ✓       | CONT      |
| 30  | Filter Envelope Attack    |          | ✓       | CONT      |
| 31  | Filter Envelope Decay     |          | ✓       | CONT      |
| 32  | Filter Envelope Sustain   |          | ✓       | CONT      |
| 33  | Filter Envelope Release   |          | ✓       | CONT      |
| 34  | Osc1 Arpeggiator Speed    |          | ✓       | CONT      |
| 35  | Osc1 Pitch/Track          |          | ✓       | CONT      |
| 36  | Osc1 Transpose            |          | ✓       | CONT      |
| 37  | Osc1 Vibrato Depth        |          | ✓       | CONT      |
| 38  | Osc1 Detune               |          | ✓       | CONT      |
| 39  | Osc1 Portamento Speed     |          | ✓       | CONT      |
| 40  | Osc1 Synchronize          |          | ✓       | BOOL      |
| 41  | Osc1 Ring Modulation      |          | ✓       | BOOL      |
| 42  | Osc1 PWM Start Value      |          | ✓       | CONT      |
| 43  | Osc1 PWM Add Value        |          | ✓       | CONT      |
| 44  | Osc1 PWM LFO Depth        |          | ✓       | CONT      |
| 45  | Osc1 Delay                |          | ✓       | CONT      |
| 46  | Osc1 Attack               |          | ✓       | CONT      |
| 47  | Osc1 Decay                |          | ✓       | CONT      |
| 48  | Osc1 Sustain              |          | ✓       | CONT      |
| 49  | Osc1 Release              |          | ✓       | CONT      |
| 50  | Osc2 Arpeggiator Speed    |          | ✓       | CONT      |
| 51  | Osc2 Pitch/Track          |          | ✓       | CONT      |
| 52  | Osc2 Transpose            |          | ✓       | CONT      |
| 53  | Osc2 Vibrato Depth        |          | ✓       | CONT      |
| 54  | Osc2 Detune               |          | ✓       | CONT      |
| 55  | Osc2 Portamento Speed     |          | ✓       | CONT      |
| 56  | Osc2 Synchronize          |          | ✓       | BOOL      |
| 57  | Osc2 Ring Modulation      |          | ✓       | BOOL      |
| 58  | Osc2 PWM Start Value      |          | ✓       | CONT      |
| 59  | Osc2 PWM Add Value        |          | ✓       | CONT      |
| 60  | Osc2 PWM LFO Depth        |          | ✓       | CONT      |
| 61  | Osc2 Delay                |          | ✓       | CONT      |
| 62  | Osc2 Attack               |          | ✓       | CONT      |
| 63  | Osc2 Decay                |          | ✓       | CONT      |
| 70  | Osc2 Sustain              |          | ✓       | CONT      |
| 71  | Osc2 Release              |          | ✓       | CONT      |
| 72  | Osc3 Arpeggiator Speed    |          | ✓       | CONT      |
| 73  | Osc3 Pitch/Track          |          | ✓       | CONT      |
| 74  | Osc3 Transpose            |          | ✓       | CONT      |
| 75  | Osc3 Vibrato Depth        |          | ✓       | CONT      |
| 76  | Osc3 Detune               |          | ✓       | CONT      |
| 77  | Osc3 Portamento Speed     |          | ✓       | CONT      |
| 78  | Osc3 Synchronize          |          | ✓       | BOOL      |
| 79  | Osc3 Ring Modulation      |          | ✓       | BOOL      |
| 80  | Osc3 PWM Start Value      |          | ✓       | CONT      |
| 81  | Osc3 PWM Add Value        |          | ✓       | CONT      |
| 82  | Osc3 PWM LFO Depth        |          | ✓       | CONT      |
| 83  | Osc3 Delay                |          | ✓       | CONT      |
| 84  | Osc3 Attack               |          | ✓       | CONT      |
| 85  | Osc3 Decay                |          | ✓       | CONT      |
| 86  | Osc3 Sustain              |          | ✓       | CONT      |
| 87  | Osc3 Release              |          | ✓       | CONT      |
| 88  | LFO1 Speed                |          | ✓       | CONT      |
| 89  | LFO1 Depth                |          | ✓       | CONT      |
| 90  | LFO1 Add Depth            |          | ✓       | CONT      |
| 91  | LFO1 Fade In              |          | ✓       | CONT      |
| 92  | LFO1 Sample And Hold      |          | ✓       | CONT      |
| 93  | LFO1 Lace Speed           |          | ✓       | CONT      |
| 94  | LFO2 Speed                |          | ✓       | CONT      |
| 95  | LFO2 Depth                |          | ✓       | CONT      |
| 102 | LFO2 Add Depth            |          | ✓       | CONT      |
| 103 | LFO2 Fade In              |          | ✓       | CONT      |
| 104 | LFO2 Sample And Hold      |          | ✓       | CONT      |
| 105 | LFO2 Lace Speed           |          | ✓       | CONT      |
| 106 | LFO3 Speed                |          | ✓       | CONT      |
| 107 | LFO3 Depth                |          | ✓       | CONT      |
| 108 | LFO3 Add Depth            |          | ✓       | CONT      |
| 109 | LFO3 Fade In              |          | ✓       | CONT      |
| 110 | LFO3 Sample And Hold      |          | ✓       | CONT      |
| 111 | LFO3 Lace Speed           |          | ✓       | CONT      |
| 112 | LFO4 Speed                |          | ✓       | CONT      |
| 113 | LFO4 Depth                |          | ✓       | CONT      |
| 114 | LFO4 Add Depth            |          | ✓       | CONT      |
| 115 | LFO4 Fade In              |          | ✓       | CONT      |
| 116 | LFO4 Sample And Hold      |          | ✓       | CONT      |
| 117 | LFO4 Lace Speed           |          | ✓       | CONT      |