# Analog Four MKI MIDI implementation

`Analog-Four-User-Manual_ENG_OS1.55_260610.pdf`: Pages 98 - 106

---

This appendix covers the MIDI CC and NRPN implementation. The data range of each parameters CC and/or NRPN value is written in parentheses after the value.

## 1. Track Parameters

### PERFORMANCE

| Parameter   | Encoder | CC MSB     | CC LSB | NRPN MSB | NRPN LSB    |
| ----------- | ------- | ---------- | ------ | -------- | ----------- |
| Mute        |         | 94 (0–127) | -      | 1        | 101 (0–127) |
| Track Level |         | 95 (0–127) | -      | 1        | 100 (0–127) |

## 2. Performance Parameters

The following messages affect the performance parameters on all tracks. They are also sent when adjusting the knobs controlling the parameters.

### PERFORMANCE

| Parameter               | Encoder | CC MSB     | CC LSB | NRPN MSB | NRPN LSB  |
| ----------------------- | ------- | ---------- | ------ | -------- | --------- |
| Performance Parameter A | A       | 3 (0–127)  | -      | 0        | 0 (0–127) |
| Performance Parameter B | B       | 4 (0–127)  | -      | 0        | 1 (0–127) |
| Performance Parameter C | C       | 8 (0–127)  | -      | 0        | 2 (0–127) |
| Performance Parameter D | D       | 9 (0–127)  | -      | 0        | 3 (0–127) |
| Performance Parameter E | E       | 11 (0–127) | -      | 0        | 4 (0–127) |
| Performance Parameter F | F       | 64 (0–127) | -      | 0        | 5 (0–127) |
| Performance Parameter G | G       | 65 (0–127) | -      | 0        | 6 (0–127) |
| Performance Parameter H | H       | 66 (0–127) | -      | 0        | 7 (0–127) |
| Performance Parameter I | I       | 67 (0–127) | -      | 0        | 8 (0–127) |
| Performance Parameter J | J       | 68 (0–127) | -      | 0        | 9 (0–127) |

## 3. Modulation Parameters

The following messages affect modulation parameters. Which parameters they will in turn affect are set in the SOUND SETTINGS menu (where modulation parameters for velocity, pitch bend and aftertouch can also be found).

### MODULATION

| Parameter         | Encoder | CC MSB    | CC LSB     | NRPN MSB | NRPN LSB |
| ----------------- | ------- | --------- | ---------- | -------- | -------- |
| Modwheel          | -       | 1 (0–127) | 33 (0–127) | -        | -        |
| Breath Controller | -       | 2 (0–127) | 34 (0–127) | -        | -        |

The following messages affect the track level parameter.

### COMMON

| Parameter   | Encoder | CC MSB     | CC LSB | NRPN MSB  | NRPN LSB |
| ----------- | ------- | ---------- | ------ | --------- | -------- |
| Track Level | -       | 95 (0–127) | -      | 1 (0–127) | 100      |

## 4. Synth Track Parameters

The following messages affect the synth track parameters. They are also sent when adjusting the knobs controlling the parameters.

### OSC 1

| Parameter      | Encoder | CC MSB     | CC LSB     | NRPN MSB  | NRPN LSB  |
| -------------- | ------- | ---------- | ---------- | --------- | --------- |
| Pitch          | A       | 16 (0–127) | 48 (0–127) | 1 (0–127) | 0 (0–127) |
|                | B       | -          | -          | 1 (0–127) |           |
| Detune         | C       | -          | -          | 1 (0–127) | 2         |
| Keytracking    | D       | -          | -          | 1 (0–127) | 3         |
| Level          | E       | 69 (0–127) | -          | 1 (0–127) | 4         |
| Waveform       | F       | 70 (0–127) | -          | 1 (0–127) | 5         |
| Sub Oscillator | G       | 71 (0–127) | -          | 1 (0–127) | 6         |
| Pulsewidth     | H       | 72 (0–127) | -          | 1 (0–127) | 7         |
| PWM Speed      | I       | 73 (0–127) | -          | 1 (0–127) |           |
| PWM Depth      | J       | 74 (0–127) | -          | 1 (0–127) | 9         |

### NOISE

| Parameter   | Encoder | CC MSB     | CC LSB | NRPN MSB  | NRPN LSB |
| ----------- | ------- | ---------- | ------ | --------- | -------- |
| Noise S&H   | A       | 75 (0–127) | -      | 1 (0–127) | 10       |
|             | B       | -          | -      | 1 (0–127) |          |
| Noise Fade  | C       | 76 (0–127) | -      | 1 (0–127) | 12       |
|             | D       | -          | -      | 1 (0–127) |          |
| Noise Level | E       | 77 (0–127) | -      | 1 (0–127) | 14       |
|             | F       | -          | -      | 1 (0–127) |          |
|             | G       | -          | -      | 1 (0–127) |          |
|             | H       | -          | -      | 1 (0–127) |          |
|             | I       | -          | -      | 1 (0–127) |          |
|             | J       | -          | -      | 1 (0–127) |          |

### OSC 2

| Parameter      | Encoder | CC MSB     | CC LSB     | NRPN MSB  | NRPN LSB   |
| -------------- | ------- | ---------- | ---------- | --------- | ---------- |
| Pitch          | A       | 17 (0–127) | 49 (0–127) | 1 (0–127) | 20 (0–127) |
|                | B       | -          | -          | 1 (0–127) |            |
| Detune         | C       | -          | -          | 1 (0–127) | 22         |
| Keytracking    | D       | -          | -          | 1 (0–127) | 23         |
| Level          | E       | 78 (0–127) | -          | 1 (0–127) | 24         |
| Waveform       | F       | 79 (0–127) | -          | 1 (0–127) | 25         |
| Sub Oscillator | G       | 80 (0–127) | -          | 1 (0–127) | 26         |
| Pulsewidth     | H       | 81 (0–127) | -          | 1 (0–127) | 27         |
| PWM Speed      | I       | 82 (0–127) | -          | 1 (0–127) |            |
| PWM Depth      | J       | 83 (0–127) | -          | 1 (0–127) | 29         |

### OSC COMMON

| Parameter     | Encoder | CC MSB     | CC LSB | NRPN MSB  | NRPN LSB |
| ------------- | ------- | ---------- | ------ | --------- | -------- |
| OSC1 AM       | A       | -          | -      | 1 (0–127) | 30       |
| Sync Mode     | B       | -          | -      | 1 (0–127) | 31       |
| Sync Amount   | C       | 84 (0–127) | -      | 1 (0–127) | 32       |
| Bend Amount   | D       | 85 (0–127) | -      | 1 (0–127) | 33       |
| Slide Time    | E       | 5 (0–127)  | -      | 1 (0–127) | 34       |
| OSC2 AM       | F       | -          | -      | 1 (0–127) | 35       |
| Note Sync     | G       | -          | -      | 1 (0–127) | 36       |
| Vibrato Fade  | H       | -          | -      | 1 (0–127) | 37       |
| Vibrato Speed | I       | 87 (0–127) | -      | 1 (0–127) | 38       |
| Vibrato Depth | J       | 88 (0–127) | -      | 1 (0–127) | 39       |

### FILTERS

| Parameter               | Encoder | CC MSB      | CC LSB     | NRPN MSB  | NRPN LSB   |
| ----------------------- | ------- | ----------- | ---------- | --------- | ---------- |
| Filter1 Frequency       | A       | 18 (0–127)  | 50 (0–127) | 1 (0–127) | 40 (0–127) |
| Filter1 Resonance       | B       | 89 (0–127)  | -          | 1 (0–127) | 41         |
| Filter Overdrive        | C       | 86 (0–127)  | -          | 1 (0–127) | 42         |
| Filter1 Keytracking     | D       | -           | -          | 1 (0–127) | 43         |
| Filter1 Envelope Amount | E       | 102 (0–127) | -          | 1 (0–127) | 44         |
| Filter2 Frequency       | F       | 19 (0–127)  | 51 (0–127) | 1 (0–127) | 45 (0–127) |
| Filter2 Resonance       | G       | 90 (0–127)  | -          | 1 (0–127) | 46         |
| Filter2 Type            | H       | -           | -          | 1 (0–127) | 47         |
| Filter2 Keytracking     | I       | -           | -          | 1 (0–127) | 48         |
| Filter2 Envelope Amount | J       | 103 (0–127) | -          | 1 (0–127) | 49         |

### AMP

| Parameter          | Encoder | CC MSB      | CC LSB | NRPN MSB  | NRPN LSB |
| ------------------ | ------- | ----------- | ------ | --------- | -------- |
| EnvA Attack Time   | A       | 104 (0–127) | -      | 1 (0–127) | 50       |
| EnvA Decay Time    | B       | 105 (0–127) | -      | 1 (0–127) | 51       |
| EnvA Sustain Level | C       | 106 (0–127) | -      | 1 (0–127) | 52       |
| EnvA Release Time  | D       | 107 (0–127) | -      | 1 (0–127) | 53       |
| EnvA Env Shape     | E       | -           | -      | 1 (0–127) | 54       |
| Chorus Send Level  | F       | 91 (0–127)  | -      | 1 (0–127) | 55       |
| Delay Send Level   | G       | 92 (0–127)  | -      | 1 (0–127) | 56       |
| Reverb Send Level  | H       | 93 (0–127)  | -      | 1 (0–127) | 57       |
| Pan                | I       | 10 (0–127)  | -      | 1 (0–127) | 58       |
| Volume             | J       | 7 (0–127)   | -      | 1 (0–127) | 59       |

### ENVF

| Parameter          | Encoder | CC MSB      | CC LSB     | NRPN MSB  | NRPN LSB   |
| ------------------ | ------- | ----------- | ---------- | --------- | ---------- |
| EnvF Attack Time   | A       | 108 (0–127) | -          | 1 (0–127) | 60         |
| EnvF Decay Time    | B       | 109 (0–127) | -          | 1 (0–127) | 61         |
| EnvF Sustain Level | C       | 110 (0–127) | -          | 1 (0–127) | 62         |
| EnvF Release Time  | D       | 111 (0–127) | -          | 1 (0–127) | 63         |
| EnvF Env Shape     | E       | -           | -          | 1 (0–127) | 64         |
| EnvF Gate Length   | F       | -           | -          | 1 (0–127) | 65         |
| EnvF Destination A | G       | -           | -          | 1 (0–127) | 66         |
| EnvF Depth A       | H       | 20 (0–127)  | 52 (0–127) | 1 (0–127) | 67 (0–127) |
| EnvF Destination B | I       | -           | -          | 1 (0–127) | 68         |
| EnvF Depth B       | J       | 21 (0–127)  | 53 (0–127) | 1 (0–127) | 69 (0–127) |

### ENV2

| Parameter          | Encoder | CC MSB      | CC LSB     | NRPN MSB  | NRPN LSB   |
| ------------------ | ------- | ----------- | ---------- | --------- | ---------- |
| Env2 Attack Time   | A       | 112 (0–127) | -          | 1 (0–127) | 70         |
| Env2 Decay Time    | B       | 113 (0–127) | -          | 1 (0–127) | 71         |
| Env2 Sustain Level | C       | 114 (0–127) | -          | 1 (0–127) | 72         |
| Env2 Release Time  | D       | 115 (0–127) | -          | 1 (0–127) | 73         |
| Env2 Env Shape     | E       | -           | -          | 1 (0–127) | 74         |
| Env2 Gate Length   | F       | -           | -          | 1 (0–127) | 75         |
| Env2 Destination A | G       | -           | -          | 1 (0–127) | 76         |
| Env2 Depth A       | H       | 22 (0–127)  | 54 (0–127) | 1 (0–127) | 77 (0–127) |
| Env2 Destination B | I       | -           | -          | 1 (0–127) | 78         |
| Env2 Depth B       | J       | 23 (0–127)  | 55 (0–127) | 1 (0–127) | 79 (0–127) |

### LFO1

| Parameter             | Encoder | CC MSB      | CC LSB     | NRPN MSB  | NRPN LSB   |
| --------------------- | ------- | ----------- | ---------- | --------- | ---------- |
| LFO1 Speed            | A       | 116 (0–127) | -          | 1 (0–127) | 80         |
| LFO1 Speed Multiplier | B       | 117 (0–127) | -          | 1 (0–127) | 81         |
| LFO1 Fade             | C       | -           | -          | 1 (0–127) | 82         |
| LFO1 Start Phase      | D       | -           | -          | 1 (0–127) | 83         |
| LFO1 Mode             | E       | -           | -          | 1 (0–127) | 84         |
| LFO1 Waveform         | F       | -           | -          | 1 (0–127) | 85         |
| LFO1 Destination 1    | G       | -           | -          | 1 (0–127) | 86         |
| LFO1 Depth 1          | H       | 24 (0–127)  | 56 (0–127) | 1 (0–127) | 87 (0–127) |
| LFO1 Destination 2    | I       | -           | -          | 1 (0–127) | 88         |
| LFO1 Depth 2          | J       | 25 (0–127)  | 57 (0–127) | 1 (0–127) | 89 (0–127) |

### LFO2

| Parameter             | Encoder | CC MSB      | CC LSB     | NRPN MSB  | NRPN LSB   |
| --------------------- | ------- | ----------- | ---------- | --------- | ---------- |
| LFO2 Speed            | A       | 118 (0–127) | -          | 1 (0–127) | 90         |
| LFO2 Speed Multiplier | B       | 119 (0–127) | -          | 1 (0–127) | 91         |
| LFO2 Fade             | C       | -           | -          | 1 (0–127) | 92         |
| LFO2 Start Phase      | D       | -           | -          | 1 (0–127) | 93         |
| LFO2 Mode             | E       | -           | -          | 1 (0–127) | 94         |
| LFO2 Waveform         | F       | -           | -          | 1 (0–127) | 95         |
| LFO2 Destination 1    | G       | -           | -          | 1 (0–127) | 96         |
| LFO2 Depth 1          | H       | 26 (0–127)  | 58 (0–127) | 1 (0–127) | 97 (0–127) |
| LFO2 Destination 2    | I       | -           | -          | 1 (0–127) | 98         |
| LFO2 Depth 2          | J       | 27 (0–127)  | 59 (0–127) | 1 (0–127) | 99 (0–127) |

## 5. FX Track Parameters

The following messages affect the FX track parameters. They are also sent when adjusting the knobs controlling the parameters.

### EXT IN

| Parameter       | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB |
| --------------- | ------- | ------ | ------ | --------- | -------- |
| Ch1 Chorus Send | A       | -      | -      | 2 (0–127) | 0        |
| Ch1 Delay Send  | B       | -      | -      | 2 (0–127) | 1        |
| Ch1 Reverb Send | C       | -      | -      | 2 (0–127) | 2        |
| Ch1 Pan         | D       | -      | -      | 2 (0–127) | 3        |
| Ch1 Level       | E       | -      | -      | 2 (0–127) | 4        |
| Ch2 Chorus Send | F       | -      | -      | 2 (0–127) | 5        |
| Ch2 Delay Send  | G       | -      | -      | 2 (0–127) | 6        |
| Ch2 Reverb Send | H       | -      | -      | 2 (0–127) | 7        |
| Ch2 Pan         | I       | -      | -      | 2 (0–127) | 8        |
| Ch2 Level       | J       | -      | -      | 2 (0–127) | 9        |

### CHORUS

| Parameter   | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB |
| ----------- | ------- | ------ | ------ | --------- | -------- |
| Predelay    | A       | -      | -      | 2 (0–127) | 40       |
| Speed       | B       | -      | -      | 2 (0–127) | 41       |
| Depth       | C       | -      | -      | 2 (0–127) | 42       |
| Width       | D       | -      | -      | 2 (0–127) | 43       |
| Feedback    | E       | -      | -      | 2 (0–127) | 44       |
| HP Filter   | F       | -      | -      | 2 (0–127) | 45       |
| LP Filter   | G       | -      | -      | 2 (0–127) | 46       |
| Delay Send  | H       | -      | -      | 2 (0–127) | 47       |
| Reverb Send | I       | -      | -      | 2 (0–127) | 48       |
| Send Level  | J       | -      | -      | 2 (0–127) | 49       |

### DELAY

| Parameter   | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB |
| ----------- | ------- | ------ | ------ | --------- | -------- |
| Time        | A       | -      | -      | 2 (0–127) | 50       |
| Mode        | B       | -      | -      | 2 (0–127) | 51       |
|             | C       | -      | -      | 2 (0–127) | 52       |
| Width       | D       | -      | -      | 2 (0–127) | 53       |
| Feedback    | E       | -      | -      | 2 (0–127) | 54       |
| HP Filter   | F       | -      | -      | 2 (0–127) | 55       |
| LP Filter   | G       | -      | -      | 2 (0–127) | 56       |
| Overdrive   | H       | -      | -      | 2 (0–127) | 57       |
| Reverb Send | I       | -      | -      | 2 (0–127) | 58       |
| Send Level  | J       | -      | -      | 2 (0–127) | 59       |

### REVERB

| Parameter     | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB |
| ------------- | ------- | ------ | ------ | --------- | -------- |
| Predelay      | A       | -      | -      | 2 (0–127) | 60       |
| Decay Time    | B       | -      | -      | 2 (0–127) | 61       |
| Shelving Freq | C       | -      | -      | 2 (0–127) | 62       |
| Shelving Gain | D       | -      | -      | 2 (0–127) | 63       |
|               | E       | -      | -      | 2 (0–127) | 64       |
| HP Filter     | F       | -      | -      | 2 (0–127) | 65       |
| LP Filter     | G       | -      | -      | 2 (0–127) | 66       |
|               | H       | -      | -      | 2 (0–127) | 67       |
|               | I       | -      | -      | 2 (0–127) | 68       |
| Send Level    | J       | -      | -      | 2 (0–127) | 69       |

### LFO1

| Parameter             | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB   |
| --------------------- | ------- | ------ | ------ | --------- | ---------- |
| LFO1 Speed            | A       | -      | -      | 2 (0–127) | 80         |
| LFO1 Speed Multiplier | B       | -      | -      | 2 (0–127) | 81         |
| LFO1 Fade             | C       | -      | -      | 2 (0–127) | 82         |
| LFO1 Start Phase      | D       | -      | -      | 2 (0–127) | 83         |
| LFO1 Mode             | E       | -      | -      | 2 (0–127) | 84         |
| LFO1 Waveform         | F       | -      | -      | 2 (0–127) | 85         |
| LFO1 Destination 1    | G       | -      | -      | 2 (0–127) | 86         |
| LFO1 Depth 1          | H       | -      | -      | 2 (0–127) | 87 (0–127) |
| LFO1 Destination 2    | I       | -      | -      | 2 (0–127) | 88         |
| LFO1 Depth 2          | J       | -      | -      | 2 (0–127) | 89 (0–127) |

### LFO2

| Parameter             | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB   |
| --------------------- | ------- | ------ | ------ | --------- | ---------- |
| LFO2 Speed            | A       | -      | -      | 2 (0–127) | 90         |
| LFO2 Speed Multiplier | B       | -      | -      | 2 (0–127) | 91         |
| LFO2 Fade             | C       | -      | -      | 2 (0–127) | 92         |
| LFO2 Start Phase      | D       | -      | -      | 2 (0–127) | 93         |
| LFO2 Mode             | E       | -      | -      | 2 (0–127) | 94         |
| LFO2 Waveform         | F       | -      | -      | 2 (0–127) | 95         |
| LFO2 Destination 1    | G       | -      | -      | 2 (0–127) | 96         |
| LFO2 Depth 1          | H       | -      | -      | 2 (0–127) | 97 (0–127) |
| LFO2 Destination 2    | I       | -      | -      | 2 (0–127) | 98         |
| LFO2 Depth 2          | J       | -      | -      | 2 (0–127) | 99 (0–127) |

## 6. CV Track Parameters

The following messages affect the CV track parameters. They are also sent when adjusting the knobs controlling the parameters.

### CV A

| Parameter            | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB |
| -------------------- | ------- | ------ | ------ | --------- | -------- |
| CV A Coarse Tune     | A       | -      | -      | 3 (0–127) | 0        |
| CV A Fine Tune       | B       | -      | -      | 3 (0–127) | 1        |
| CV A Value           | C       | -      | -      | 3 (0–127) | 2        |
| CV A Clock           | D       | -      | -      | 3 (0–127) | 3        |
| CV A Source          | E       | -      | -      | 3 (0–127) | 4        |
| CV A Bend Depth      | F       | -      | -      | 3 (0–127) | 5        |
| CV A Note Slide time | G       | -      | -      | 3 (0–127) | 6        |
|                      | H       | -      | -      | 3 (0–127) | -        |
|                      | I       | -      | -      | 3 (0–127) | -        |
|                      | J       | -      | -      | 3 (0–127) | -        |

### CV B

| Parameter            | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB |
| -------------------- | ------- | ------ | ------ | --------- | -------- |
| CV B Coarse Tune     | A       | -      | -      | 3 (0–127) | 20       |
| CV B Fine Tune       | B       | -      | -      | 3 (0–127) | 21       |
| CV B Value           | C       | -      | -      | 3 (0–127) | 22       |
| CV B Clock           | D       | -      | -      | 3 (0–127) | 23       |
| CV B Source          | E       | -      | -      | 3 (0–127) | 24       |
| CV B Bend Depth      | F       | -      | -      | 3 (0–127) | 25       |
| CV B Note Slide time | G       | -      | -      | 3 (0–127) | 26       |
|                      | H       | -      | -      | 3 (0–127) | -        |
|                      | I       | -      | -      | 3 (0–127) | -        |
|                      | J       | -      | -      | 3 (0–127) | -        |

### CV C

| Parameter            | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB |
| -------------------- | ------- | ------ | ------ | --------- | -------- |
| CV C Coarse Tune     | A       | -      | -      | 3 (0–127) | 40       |
| CV C Fine Tune       | B       | -      | -      | 3 (0–127) | 41       |
| CV C Value           | C       | -      | -      | 3 (0–127) | 42       |
| CV C Clock           | D       | -      | -      | 3 (0–127) | 43       |
| CV C Source          | E       | -      | -      | 3 (0–127) | 44       |
| CV C Bend Depth      | F       | -      | -      | 3 (0–127) | 45       |
| CV C Note Slide time | G       | -      | -      | 3 (0–127) | 46       |
|                      | H       | -      | -      | 3 (0–127) | -        |
|                      | I       | -      | -      | 3 (0–127) | -        |
|                      | J       | -      | -      | 3 (0–127) | -        |

### CV D

| Parameter            | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB |
| -------------------- | ------- | ------ | ------ | --------- | -------- |
| CV D Coarse Tune     | A       | -      | -      | 3 (0–127) | 50       |
| CV D Fine Tune       | B       | -      | -      | 3 (0–127) | 51       |
| CV D Value           | C       | -      | -      | 3 (0–127) | 52       |
| CV D Clock           | D       | -      | -      | 3 (0–127) | 53       |
| CV D Source          | E       | -      | -      | 3 (0–127) | 54       |
| CV D Bend Depth      | F       | -      | -      | 3 (0–127) | 55       |
| CV D Note Slide time | G       | -      | -      | 3 (0–127) | 56       |
|                      | H       | -      | -      | 3 (0–127) | -        |
|                      | I       | -      | -      | 3 (0–127) | -        |
|                      | J       | -      | -      | 3 (0–127) | -        |

### ENV1

| Parameter          | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB   |
| ------------------ | ------- | ------ | ------ | --------- | ---------- |
| Env1 Attack Time   | A       | -      | -      | 3 (0–127) | 60         |
| Env1 Decay Time    | B       | -      | -      | 3 (0–127) | 61         |
| Env1 Sustain Level | C       | -      | -      | 3 (0–127) | 62         |
| Env1 Release Time  | D       | -      | -      | 3 (0–127) | 63         |
| Env1 Env Shape     | E       | -      | -      | 3 (0–127) | 64         |
| Env1 Gate Length   | F       | -      | -      | 3 (0–127) | 65         |
| Env1 Destination 1 | G       | -      | -      | 3 (0–127) | 66         |
| Env1 Depth 1       | H       | -      | -      | 3 (0–127) | 67 (0–127) |
| Env1 Destination 2 | I       | -      | -      | 3 (0–127) | 68         |
| Env1 Depth 2       | J       | -      | -      | 3 (0–127) | 69 (0–127) |

### ENV2

| Parameter          | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB   |
| ------------------ | ------- | ------ | ------ | --------- | ---------- |
| Env2 Attack Time   | A       | -      | -      | 3 (0–127) | 70         |
| Env2 Decay Time    | B       | -      | -      | 3 (0–127) | 71         |
| Env2 Sustain Level | C       | -      | -      | 3 (0–127) | 72         |
| Env2 Release Time  | D       | -      | -      | 3 (0–127) | 73         |
| Env2 Env Shape     | E       | -      | -      | 3 (0–127) | 74         |
| Env2 Gate Length   | F       | -      | -      | 3 (0–127) | 75         |
| Env2 Destination 1 | G       | -      | -      | 3 (0–127) | 76         |
| Env2 Depth 1       | H       | -      | -      | 3 (0–127) | 77 (0–127) |
| Env2 Destination 2 | I       | -      | -      | 3 (0–127) | 78         |
| Env2 Depth 2       | J       | -      | -      | 3 (0–127) | 79 (0–127) |

### LFO1

| Parameter             | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB   |
| --------------------- | ------- | ------ | ------ | --------- | ---------- |
| LFO1 Speed            | A       | -      | -      | 3 (0–127) | 80         |
| LFO1 Speed Multiplier | B       | -      | -      | 3 (0–127) | 81         |
| LFO1 Fade             | C       | -      | -      | 3 (0–127) | 82         |
| LFO1 Start Phase      | D       | -      | -      | 3 (0–127) | 83         |
| LFO1 Mode             | E       | -      | -      | 3 (0–127) | 84         |
| LFO1 Waveform         | F       | -      | -      | 3 (0–127) | 85         |
| LFO1 Destination 1    | G       | -      | -      | 3 (0–127) | 86         |
| LFO1 Depth 1          | H       | -      | -      | 3 (0–127) | 87 (0–127) |
| LFO1 Destination 2    | I       | -      | -      | 3 (0–127) | 88         |
| LFO1 Depth 2          | J       | -      | -      | 3 (0–127) | 89 (0–127) |

### LFO2

| Parameter             | Encoder | CC MSB | CC LSB | NRPN MSB  | NRPN LSB   |
| --------------------- | ------- | ------ | ------ | --------- | ---------- |
| LFO2 Speed            | A       | -      | -      | 3 (0–127) | 90         |
| LFO2 Speed Multiplier | B       | -      | -      | 3 (0–127) | 91         |
| LFO2 Fade             | C       | -      | -      | 3 (0–127) | 92         |
| LFO2 Start Phase      | D       | -      | -      | 3 (0–127) | 93         |
| LFO2 Mode             | E       | -      | -      | 3 (0–127) | 94         |
| LFO2 Waveform         | F       | -      | -      | 3 (0–127) | 95         |
| LFO2 Destination 1    | G       | -      | -      | 3 (0–127) | 96         |
| LFO2 Depth 1          | H       | -      | -      | 3 (0–127) | 97 (0–127) |
| LFO2 Destination 2    | I       | -      | -      | 3 (0–127) | 98         |
| LFO2 Depth 2          | J       | -      | -      | 3 (0–127) | 99 (0–127) |
