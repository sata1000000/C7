# Analog Rytm MKI MIDI implementation

`Analog-Rytm-User-Manual_ENG_OS1.72_250130.pdf`: Pages 79 - 89

---

This appendix lists the CC and NRPN specification for the Analog Rytm.

## 1. General Trig Parameters

### TRIG PARAMETERS

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Note        | 3      | -      | 3        | 0        |
| Velocity    | 4      | -      | 3        | 1        |
| Length      | 5      | -      | 3        | 2        |
| Synth Trig  | 11     | -      | 3        | 3        |
| Sample Trig | 12     | -      | 3        | 4        |
| ENV Trig    | 13     | -      | 3        | 5        |
| LFO Trig    | 14     | -      | 3        | 6        |

## 2. Euclidean Sequencer Parameters

### EUCLIDEAN PARAMETERS

| Parameter            | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| -------------------- | ------ | ------ | -------- | -------- |
| Pulse Generator 1    | 86     | -      | 3        | 8        |
| Pulse Generator 2    | 87     | -      | 3        | 9        |
| Euclidean on/off     | 117    | -      | 3        | 14       |
| Rotation Generator 1 | 89     | -      | 3        | 11       |
| Rotation Generator 2 | 90     | -      | 3        | 12       |
| Track Rotation       | 91     | -      | 3        | 13       |
| Boolean Operator     | 88     | -      | 3        | 10       |

## 3. General Kit Parameters

### COMMON

| Parameter              | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ---------------------- | ------ | ------ | -------- | -------- |
| Track Level            | 95     | -      | 1        | 100      |
| Track Mute (seq. mute) | 94     | -      | 1        | 101      |
| Track Solo (seq. mute) | 93     | -      | 1        | 102      |
| Track Machine Type     | 15     | -      | 1        | 103      |
| Active Scene           | 92     | -      | 1        | 104      |

## 4. Performance Parameters

### PERFORMANCE

| Parameter                | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ------------------------ | ------ | ------ | -------- | -------- |
| Performance Parameter 1  | 35     | -      | 0        | 0        |
| Performance Parameter 2  | 36     | -      | 0        | 1        |
| Performance Parameter 3  | 37     | -      | 0        | 2        |
| Performance Parameter 4  | 39     | -      | 0        | 3        |
| Performance Parameter 5  | 40     | -      | 0        | 4        |
| Performance Parameter 6  | 41     | -      | 0        | 5        |
| Performance Parameter 7  | 42     | -      | 0        | 6        |
| Performance Parameter 8  | 43     | -      | 0        | 7        |
| Performance Parameter 9  | 44     | -      | 0        | 8        |
| Performance Parameter 10 | 45     | -      | 0        | 9        |
| Performance Parameter 11 | 46     | -      | 0        | 10       |
| Performance Parameter 12 | 47     | -      | 0        | 11       |

## 5. General Synth Parameters

Note that the order of the SYNTH parameters below will sometimes differ from the order of the parameters shown on the SRC page of the RYTM. See the section MACHINE PARAMETERS below for a more detailed list.

### SYNTH

| Parameter         | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------------- | ------ | ------ | -------- | -------- |
| Synth Parameter 1 | 16     | -      | 1        | 0        |
| Synth Parameter 2 | 17     | -      | 1        | 1        |
| Synth Parameter 3 | 18     | -      | 1        | 2        |
| Synth Parameter 4 | 19     | -      | 1        | 3        |
| Synth Parameter 5 | 20     | -      | 1        | 4        |
| Synth Parameter 6 | 21     | -      | 1        | 5        |
| Synth Parameter 7 | 22     | -      | 1        | 6        |
| Synth Parameter 8 | 23     | -      | 1        | 7        |

### SAMPLE

| Parameter            | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| -------------------- | ------ | ------ | -------- | -------- |
| Sample Tune          | 24     | -      | 1        | 8        |
| Sample Fine tune     | 25     | -      | 1        | 9        |
| Sample Bit Reduction | 26     | -      | 1        | 10       |
| Sample Slot          | 27     | -      | 1        | 11       |
| Sample Start         | 28     | -      | 1        | 12       |
| Sample End           | 29     | -      | 1        | 13       |
| Sample Loop          | 30     | -      | 1        | 14       |
| Sample Level         | 31     | -      | 1        | 15       |

### FILTER

| Parameter            | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| -------------------- | ------ | ------ | -------- | -------- |
| Filter Attack Time   | 70     | -      | 1        | 16       |
| Filter Decay Time    | 71     | -      | 1        | 17       |
| Filter Sustain Level | 72     | -      | 1        | 18       |
| Filter Release Time  | 73     | -      | 1        | 19       |
| Filter Frequency     | 74     | -      | 1        | 20       |
| Filter Resonance     | 75     | -      | 1        | 21       |
| Filter Mode          | 76     | -      | 1        | 22       |
| Filter Env Depth     | 77     | -      | 1        | 23       |

### AMP

| Parameter       | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------------- | ------ | ------ | -------- | -------- |
| Amp Attack Time | 78     | -      | 1        | 24       |
| Amp Hold Time   | 79     | -      | 1        | 25       |
| Amp Decay Time  | 80     | -      | 1        | 26       |
| Amp Overdrive   | 81     | -      | 1        | 27       |
| Amp Delay Send  | 82     | -      | 1        | 28       |
| Amp Reverb Send | 83     | -      | 1        | 29       |
| Amp Pan         | 10     | -      | 1        | 30       |
| Amp Volume      | 7      | -      | 1        | 31       |

## 6. LFO Parameters

Note that the LFO depth is a high-resolution parameter, with CC LSB value.

### LFO

| Parameter       | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------------- | ------ | ------ | -------- | -------- |
| LFO Speed       | 102    | -      | 1        | 32       |
| LFO Multiplier  | 103    | -      | 1        | 33       |
| LFO Fade In/Out | 104    | -      | 1        | 34       |
| LFO Destination | 105    | -      | 1        | 35       |
| LFO Waveform    | 106    | -      | 1        | 36       |
| LFO Start Phase | 107    | -      | 1        | 37       |
| LFO Trig Mode   | 108    | -      | 1        | 38       |
| LFO Depth       | 109    | 118    | 1        | 39       |

## 7. FX Parameters

### DELAY

| Parameter             | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------------------- | ------ | ------ | -------- | -------- |
| Delay Time            | 16     | -      | 2        | 0        |
| Delay Pingpong        | 17     | -      | 2        | 1        |
| Delay Stereo Width    | 18     | -      | 2        | 2        |
| Delay Feedback        | 19     | -      | 2        | 3        |
| Delay Highpass Filter | 20     | -      | 2        | 4        |
| Delay Lowpass Filter  | 21     | -      | 2        | 5        |
| Delay Reverb Send     | 22     | -      | 2        | 6        |
| Delay Mix Volume      | 23     | -      | 2        | 7        |

### REVERB

| Parameter              | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ---------------------- | ------ | ------ | -------- | -------- |
| Reverb Predelay        | 24     | -      | 2        | 8        |
| Reverb Decay Time      | 25     | -      | 2        | 9        |
| Reverb Shelving Freq   | 26     | -      | 2        | 10       |
| Reverb Shelving Gain   | 27     | -      | 2        | 11       |
| Reverb Highpass Filter | 28     | -      | 2        | 12       |
| Reverb Lowpass Filter  | 29     | -      | 2        | 13       |
| Reverb Mix Volume      | 31     | -      | 2        | 15       |

### DISTORTION

| Parameter                           | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------------------------------- | ------ | ------ | -------- | -------- |
| Dist Amount                         | 70     | -      | 2        | 16       |
| Dist Symmetry                       | 71     | -      | 2        | 17       |
| Delay Overdrive                     | 72     | -      | 2        | 18       |
| Delay Dist/Comp Routing (pre/post)  | 76     | -      | 2        | 22       |
| Reverb Dist/Comp Routing (pre/post) | 77     | -      | 2        | 23       |

### COMPRESSOR

| Parameter                | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ------------------------ | ------ | ------ | -------- | -------- |
| Compressor Threshold     | 78     | -      | 2        | 24       |
| Compressor Attack Time   | 79     | -      | 2        | 25       |
| Compressor Release Time  | 80     | -      | 2        | 26       |
| Compressor Makeup Gain   | 81     | -      | 2        | 27       |
| Compressor Ratio         | 82     | -      | 2        | 28       |
| Compressor Sidechain EQ  | 83     | -      | 2        | 29       |
| Compressor Dry/Wet Mix   | 84     | -      | 2        | 30       |
| Compressor Output Volume | 85     | -      | 2        | 31       |

## 8. Machine Parameters

The following shows the SYNTH parameters per MACHINE type.

### BD PLASTIC

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay Time  | 18     | -      | 1        | 2        |
| Sweep Depth | 19     | -      | 1        | 3        |
| Sweep Time  | 20     | -      | 1        | 4        |
| Hold Time   | 21     | -      | 1        | 5        |
| VCO Click   | 22     | -      | 1        | 6        |
| Dust Level  | 23     | -      | 1        | 7        |

### BD SHARP

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Sweep Depth | 19     | -      | 1        | 3        |
| Sweep Time  | 20     | -      | 1        | 4        |
| Hold Time   | 21     | -      | 1        | 5        |
| Tick Level  | 22     | -      | 1        | 6        |
| Waveform    | 23     | -      | 1        | 7        |

### BD HARD

| Parameter      | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| -------------- | ------ | ------ | -------- | -------- |
| Level          | 16     | -      | 1        | 0        |
| Tune           | 17     | -      | 1        | 1        |
| Decay          | 18     | -      | 1        | 2        |
| Hold           | 19     | -      | 1        | 3        |
| Sweep Time     | 20     | -      | 1        | 4        |
| Sweep Depth    | 21     | -      | 1        | 5        |
| Waveform       | 22     | -      | 1        | 6        |
| Transient Tick | 23     | -      | 1        | 7        |

### BD CLASSIC

| Parameter      | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| -------------- | ------ | ------ | -------- | -------- |
| Level          | 16     | -      | 1        | 0        |
| Tune           | 17     | -      | 1        | 1        |
| Decay          | 18     | -      | 1        | 2        |
| Hold           | 19     | -      | 1        | 3        |
| Sweep Time     | 20     | -      | 1        | 4        |
| Sweep Depth    | 21     | -      | 1        | 5        |
| Waveform       | 22     | -      | 1        | 6        |
| Transient Tick | 23     | -      | 1        | 7        |

### BD FM

| Parameter     | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ------------- | ------ | ------ | -------- | -------- |
| Level         | 16     | -      | 1        | 0        |
| Tune          | 17     | -      | 1        | 1        |
| Decay         | 18     | -      | 1        | 2        |
| FM Amount     | 19     | -      | 1        | 3        |
| Sweep Time    | 20     | -      | 1        | 4        |
| FM Sweep Time | 21     | -      | 1        | 5        |
| FM Decay Time | 22     | -      | 1        | 6        |
| FM Tune       | 23     | -      | 1        | 7        |

### BD SILKY

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Sweep Depth | 19     | -      | 1        | 3        |
| Sweep Time  | 20     | -      | 1        | 4        |
| Hold        | 21     | -      | 1        | 5        |
| VCO Click   | 22     | -      | 1        | 6        |
| Dust Level  | 23     | -      | 1        | 7        |

### BD ACOUSTIC

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Sweep Depth | 19     | -      | 1        | 3        |
| Sweep Time  | 20     | -      | 1        | 4        |
| Hold Time   | 21     | -      | 1        | 5        |
| Impact      | 22     | -      | 1        | 6        |
| Waveform    | 23     | -      | 1        | 7        |

### SD NATURAL

| Parameter       | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------------- | ------ | ------ | -------- | -------- |
| Level           | 16     | -      | 1        | 0        |
| Tune            | 17     | -      | 1        | 1        |
| Body Decay      | 18     | -      | 1        | 2        |
| Noise Decay     | 19     | -      | 1        | 3        |
| Noise LPF       | 20     | -      | 1        | 4        |
| Noise Balance   | 21     | -      | 1        | 5        |
| Noise Resonance | 22     | -      | 1        | 6        |
| Noise HPF       | 23     | -      | 1        | 7        |

### SD HARD

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Sweep Depth | 19     | -      | 1        | 3        |
| Tick Level  | 20     | -      | 1        | 4        |
| Noise Decay | 21     | -      | 1        | 5        |
| Noise Level | 22     | -      | 1        | 6        |
| Sweep Time  | 23     | -      | 1        | 7        |

### SD CLASSIC

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Detune      | 19     | -      | 1        | 3        |
| Snap Amount | 20     | -      | 1        | 4        |
| Noise Decay | 21     | -      | 1        | 5        |
| Noise Level | 22     | -      | 1        | 6        |
| Osc Balance | 23     | -      | 1        | 7        |

### SD FM

| Parameter     | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ------------- | ------ | ------ | -------- | -------- |
| Level         | 16     | -      | 1        | 0        |
| Tune          | 17     | -      | 1        | 1        |
| Decay         | 18     | -      | 1        | 2        |
| FM Tune       | 19     | -      | 1        | 3        |
| FM Decay Time | 20     | -      | 1        | 4        |
| Noise Decay   | 21     | -      | 1        | 5        |
| Noise Level   | 22     | -      | 1        | 6        |
| FM Amount     | 23     | -      | 1        | 7        |

### SD ACOUSTIC

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Noise Decay | 19     | -      | 1        | 3        |
| Hold Time   | 20     | -      | 1        | 4        |
| Noise Level | 21     | -      | 1        | 5        |
| Impact      | 22     | -      | 1        | 6        |
| Sweep Depth | 23     | -      | 1        | 7        |

### RS HARD

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Sweep Depth | 19     | -      | 1        | 3        |
| Tick Level  | 20     | -      | 1        | 4        |
| Noise Level | 21     | -      | 1        | 5        |
| Symmetry    | 22     | -      | 1        | 6        |
| Sweep Time  | 23     | -      | 1        | 7        |

### RS CLASSIC

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune Osc 1  | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Osc Balance | 19     | -      | 1        | 3        |
| Tune Osc 2  | 20     | -      | 1        | 4        |
| Symmetry    | 21     | -      | 1        | 5        |
| Noise Level | 22     | -      | 1        | 6        |
| Tick Level  | 23     | -      | 1        | 7        |

### CP CLASSIC

| Parameter    | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ------------ | ------ | ------ | -------- | -------- |
| Level        | 16     | -      | 1        | 0        |
| Noise Tone   | 17     | -      | 1        | 1        |
| Noise Decay  | 18     | -      | 1        | 2        |
| Clap Number  | 19     | -      | 1        | 3        |
| Clap Rate    | 20     | -      | 1        | 4        |
| Noise Level  | 21     | -      | 1        | 5        |
| Random Claps | 22     | -      | 1        | 6        |
| Clap Decay   | 23     | -      | 1        | 7        |

### SY DUAL VCO

| Parameter    | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ------------ | ------ | ------ | -------- | -------- |
| Level        | 16     | -      | -        | -        |
| Osc 1 Tune   | 17     | -      | -        | -        |
| Osc 1 Decay  | 18     | -      | -        | -        |
| Balance      | 19     | -      | -        | -        |
| Osc 2 Detune | 20     | -      | -        | -        |
| Osc Config   | 21     | -      | -        | -        |
| Osc 2 Decay  | 22     | -      | -        | -        |
| Bend         | 23     | -      | -        | -        |

### SY CHIP

| Parameter | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------- | ------ | ------ | -------- | -------- |
| Level     | 16     | -      | -        | -        |
| Tune      | 17     | -      | -        | -        |
| Decay     | 18     | -      | -        | -        |
| Waveform  | 19     | -      | -        | -        |
| Speed     | 20     | -      | -        | -        |
| Offset 2  | 21     | -      | -        | -        |
| Offset 3  | 22     | -      | -        | -        |
| Offset 4  | 23     | -      | -        | -        |

### SY RAW

| Parameter    | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ------------ | ------ | ------ | -------- | -------- |
| Level        | 16     | -      | -        | -        |
| Tune         | 17     | -      | -        | -        |
| Decay        | 18     | -      | -        | -        |
| Noise Level  | 19     | -      | -        | -        |
| Osc 2 Detune | 20     | -      | -        | -        |
| Waveform 1   | 21     | -      | -        | -        |
| Waveform 2   | 22     | -      | -        | -        |
| Balance      | 23     | -      | -        | -        |

### BT CLASSIC

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Sweep Depth | 19     | -      | 1        | 3        |
| Noise Level | 20     | -      | 1        | 4        |
| Snap Type   | 21     | -      | 1        | 5        |

### LT, MT, HT CLASSIC

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Decay       | 18     | -      | 1        | 2        |
| Sweep Depth | 19     | -      | 1        | 3        |
| Sweep Time  | 20     | -      | 1        | 4        |
| Noise Decay | 21     | -      | 1        | 5        |
| Noise Level | 22     | -      | 1        | 6        |
| Noise Tone  | 23     | -      | 1        | 7        |

### CH CLASSIC

| Parameter | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------- | ------ | ------ | -------- | -------- |
| Level     | 16     | -      | 1        | 0        |
| Tune      | 17     | -      | 1        | 1        |
| Decay     | 18     | -      | 1        | 2        |
| Color     | 19     | -      | 1        | 3        |

### CH METALLIC

| Parameter  | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ---------- | ------ | ------ | -------- | -------- |
| Level      | 16     | -      | 1        | 0        |
| Tune       | 17     | -      | 1        | 1        |
| Decay Time | 18     | -      | 1        | 2        |

### OH CLASSIC

| Parameter | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------- | ------ | ------ | -------- | -------- |
| Level     | 16     | -      | 1        | 0        |
| Tune      | 17     | -      | 1        | 1        |
| Decay     | 18     | -      | 1        | 2        |
| Color     | 19     | -      | 1        | 3        |

### OH METALLIC

| Parameter  | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ---------- | ------ | ------ | -------- | -------- |
| Level      | 16     | -      | 1        | 0        |
| Tune       | 17     | -      | 1        | 1        |
| Decay Time | 18     | -      | 1        | 2        |

### HH BASIC

| Parameter       | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------------- | ------ | ------ | -------- | -------- |
| Level           | 16     | -      | 1        | 0        |
| Tune            | 17     | -      | 1        | 1        |
| Decay Time      | 18     | -      | 1        | 2        |
| Tone            | 19     | -      | 1        | 3        |
| Transient Decay | 20     | -      | 1        | 4        |
| Osc Reset       | 21     | -      | 1        | 5        |

### HH LAB

| Parameter  | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ---------- | ------ | ------ | -------- | -------- |
| Level      | 16     | -      | 1        | 0        |
| Tune 1     | 17     | -      | 1        | 1        |
| Decay Time | 18     | -      | 1        | 2        |
| Tune 2     | 19     | -      | 1        | 3        |
| Tune 3     | 20     | -      | 1        | 3        |
| Tune 4     | 21     | -      | 1        | 3        |
| Tune 5     | 22     | -      | 1        | 4        |
| Tune 6     | 23     | -      | 1        | 5        |

### CY METALLIC

| Parameter       | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------------- | ------ | ------ | -------- | -------- |
| Level           | 16     | -      | 1        | 0        |
| Tune            | 17     | -      | 1        | 1        |
| Decay Time      | 18     | -      | 1        | 2        |
| Tone            | 19     | -      | 1        | 3        |
| Transient Decay | 20     | -      | 1        | 4        |

### CY CLASSIC

| Parameter | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------- | ------ | ------ | -------- | -------- |
| Level     | 16     | -      | 1        | 0        |
| Tune      | 17     | -      | 1        | 1        |
| Decay     | 18     | -      | 1        | 2        |
| Color     | 19     | -      | 1        | 3        |
| Tone      | 20     | -      | 1        | 4        |

### CY RIDE

| Parameter   | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ----------- | ------ | ------ | -------- | -------- |
| Level       | 16     | -      | 1        | 0        |
| Tune        | 17     | -      | 1        | 1        |
| Tail Decay  | 18     | -      | 1        | 2        |
| Hit Decay   | 19     | -      | 1        | 3        |
| Cymbal Type | 20     | -      | 1        | 4        |
| Component 1 | 21     | -      | 1        | 5        |
| Component 2 | 22     | -      | 1        | 6        |
| Component 3 | 23     | -      | 1        | 7        |

### CB CLASSIC & METALLIC

| Parameter  | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ---------- | ------ | ------ | -------- | -------- |
| Level      | 16     | -      | 1        | 0        |
| Tune       | 17     | -      | 1        | 1        |
| Decay Time | 18     | -      | 1        | 2        |
| Detune     | 19     | -      | 1        | 3        |

### UT NOISE

| Parameter    | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| ------------ | ------ | ------ | -------- | -------- |
| Level        | 16     | -      | 1        | 0        |
| LP Frequency | 17     | -      | 1        | 1        |
| Decay        | 18     | -      | 1        | 2        |
| Sweep Depth  | 19     | -      | 1        | 3        |
| Sweep Time   | 20     | -      | 1        | 4        |
| LP Resonance | 21     | -      | 1        | 5        |
| HP Frequency | 22     | -      | 1        | 6        |
| Attack       | 23     | -      | 1        | 7        |

### UT IMPULSE

| Parameter | CC MSB | CC LSB | NRPN MSB | NRPN LSB |
| --------- | ------ | ------ | -------- | -------- |
| Level     | 16     | -      | 1        | 0        |
| Attack    | 17     | -      | 1        | 1        |
| Decay     | 18     | -      | 1        | 2        |
| Polarity  | 19     | -      | 1        | 3        |

## 9. MIDI Note Triggers

Some functions of the Analog Rytm can be triggered by sending MIDI note values from an external MIDI device.

Note that the tracks needs to be set to their default channels 1–12 to be triggered by C0–B0. For more information, please see "8.6 MIDI NOTES" on page 22.

| Note              | Function                                |
| ----------------- | --------------------------------------- |
| C0 (0)            | Triggers Sound Track 1                  |
| C#0 (1)           | Triggers Sound Track 2                  |
| D0 (2)            | Triggers Sound Track 3                  |
| D#0 (3)           | Triggers Sound Track 4                  |
| E0 (4)            | Triggers Sound Track 5                  |
| F0 (5)            | Triggers Sound Track 6                  |
| F#0 (6)           | Triggers Sound Track 7                  |
| G0 (7)            | Triggers Sound Track 8                  |
| G#0 (8)           | Triggers Sound Track 9                  |
| A0 (9)            | Triggers Sound Track 10                 |
| A#0 (10)          | Triggers Sound Track 11                 |
| B0 (11)           | Triggers Sound Track 12                 |
| C1 (12) – B4 (59) | Triggers the active track chromatically |
