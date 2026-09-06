# Machinedrum SysEx implementation

`machinedrum_manual_OS1.63.pdf`: Pages 115 - 120

---

This appendix lists all Machinedrum SYSEX messages available for external control.

Data printed with a `$` sign is written in hexadecimal format. Data printed with a `%` sign is written in a binary bitfield format.

## 1. Machinedrum SysEx Messages

All SYSEX messages start with this sequence.

| MIDI Byte | Purpose                |
| --------- | ---------------------- |
| `$f0`     | SYSEX Start            |
| `$00`     | Europe/USA ID          |
| `$20`     | Europe ID              |
| `$3c`     | Elektron ESI ID        |
| `$02`     | Machinedrum ID         |
| `$00`     | Base channel (Padding) |

A complete SYSEX message looks like this:

`$f0,$00,$20,$3c,$02,$00,command,...,$f7`

### SYSEX global settings dump

| MIDI Byte    | Purpose                   |
| ------------ | ------------------------- |
| (SYSEX init) |                           |
| `$50`        | Global settings dump ID   |
| ...          | Global setting data bytes |
| `$f7`        | SYSEX end                 |

### SYSEX global setting dump request

| MIDI Byte    | Purpose                                       |
| ------------ | --------------------------------------------- |
| (SYSEX init) |                                               |
| `$51`        | Global setting dump request ID                |
| `%0000aaaa`  | Send global setting `%aaaa` (0 to 7) by sysex |
| `$f7`        | SYSEX end                                     |

### SYSEX kit dump

| MIDI Byte    | Purpose        |
| ------------ | -------------- |
| (SYSEX init) |                |
| `$52`        | Kit dump ID    |
| ...          | Kit data bytes |
| `$f7`        | SYSEX end      |

### SYSEX kit request

| MIDI Byte    | Purpose                                      |
| ------------ | -------------------------------------------- |
| (SYSEX init) |                                              |
| `$53`        | Kit dump request ID                          |
| `%00aaaaaa`  | Send kit number `%aaaaaa` (0 to 63) by sysex |
| `$f7`        | SYSEX end                                    |

### SYSEX unused (`$54`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$54`        | None      |
| `$f7`        | SYSEX end |

### SYSEX set current kit name

| MIDI Byte    | Purpose                 |
| ------------ | ----------------------- |
| (SYSEX init) |                         |
| `$55`        | Set current kit name ID |
| ...          | 16 bytes ASCII (7-bit)  |
| `$f7`        | SYSEX end               |

### SYSEX set active global setting

| MIDI Byte    | Purpose                                   |
| ------------ | ----------------------------------------- |
| (SYSEX init) |                                           |
| `$56`        | Set active global setting ID              |
| `%0000aaaa`  | Set global pos `%aaaa` (0 to 7) as active |
| `$f7`        | SYSEX end                                 |

### SYSEX load pattern

| MIDI Byte    | Purpose                            |
| ------------ | ---------------------------------- |
| (SYSEX init) |                                    |
| `$57`        | Load pattern ID                    |
| `%0aaaaaaa`  | Load pattern `%aaaaaaa` (0 to 127) |
| `$f7`        | SYSEX end                          |

### SYSEX load kit

| MIDI Byte    | Purpose                      |
| ------------ | ---------------------------- |
| (SYSEX init) |                              |
| `$58`        | Load kit ID                  |
| `%00aaaaaa`  | Load kit `%aaaaaa` (0 to 63) |
| `$f7`        | SYSEX end                    |

### SYSEX save kit

| MIDI Byte    | Purpose                             |
| ------------ | ----------------------------------- |
| (SYSEX init) |                                     |
| `$59`        | Save kit ID                         |
| `%00aaaaaa`  | Save kit to pos `%aaaaaa` (0 to 63) |
| `$f7`        | SYSEX end                           |

### SYSEX set midi note to track mapping

| MIDI Byte    | Purpose                              |
| ------------ | ------------------------------------ |
| (SYSEX init) |                                      |
| `$5a`        | Set midi note mapping ID             |
| `%0aaaaaaa`  | Associate note `%aaaaaaa` ($0=C0)... |
| `%0000bbbb`  | ...with track `%bbbb` (0 to 15)      |
| `$f7`        | SYSEX end                            |

### SYSEX assign machine

| MIDI Byte    | Purpose                                                                                                       |
| ------------ | ------------------------------------------------------------------------------------------------------------- |
| (SYSEX init) |                                                                                                               |
| `$5b`        | Load machine ID                                                                                               |
| `%0000aaaa`  | Select track `%aaaa` (0 to 15)...                                                                             |
| `%0bbbbbbb`  | ...and assign machine `%bbbbbbb` number (see list below)                                                      |
| `%0000000c`  | 0 = SPS-1, 1 = SPS-1UW (optional parameter)                                                                   |
| `%000000dd`  | 0 = init synthesis, 1 = init synthesis + effects, 2 = init synthesis + effects + routing (optional parameter) |
| `$f7`        | SYSEX end                                                                                                     |

If c = 0 (SPS-1 / SPS-1UW machines):

| ID | Machine   | ID | Machine | ID | Machine | ID | Machine |
| -- | --------- | -- | ------- | -- | ------- | -- | ------- |
| 00 | GND-EMPTY | 16 | TRX-BD  | 32 | EFM-BD  | 48 | E12-BD  |
| 01 | GND-SIN   | 17 | TRX-SD  | 33 | EFM-SD  | 49 | E12-SD  |
| 02 | GND-NS    | 18 | TRX-XT  | 34 | EFM-XT  | 50 | E12-HT  |
| 03 | GND-IM    | 19 | TRX-CP  | 35 | EFM-CP  | 51 | E12-LT  |
|    |           | 20 | TRX-RS  | 36 | EFM-RS  | 52 | E12-CP  |
|    |           | 21 | TRX-CB  | 37 | EFM-CB  | 53 | E12-RS  |
|    |           | 22 | TRX-CH  | 38 | EFM-HH  | 54 | E12-CB  |
|    |           | 23 | TRX-OH  | 39 | EFM-CY  | 55 | E12-CH  |
|    |           | 24 | TRX-CY  |    |         | 56 | E12-OH  |
|    |           | 25 | TRX-MA  |    |         | 57 | E12-RC  |
|    |           | 26 | TRX-CL  |    |         | 58 | E12-CC  |
|    |           | 27 | TRX-XC  |    |         | 59 | E12-BR  |
|    |           | 28 | TRX-B2  |    |         | 60 | E12-TA  |
|    |           |    |         |    |         | 61 | E12-TR  |
|    |           |    |         |    |         | 62 | E12-SH  |
|    |           |    |         |    |         | 63 | E12-BC  |

| ID | Machine | ID | Machine | ID  | Machine | ID  | Machine |
| -- | ------- | -- | ------- | --- | ------- | --- | ------- |
| 64 | P-I-BD  | 80 | INP-GA  | 96  | MID-01  | 112 | CTR-AL  |
| 65 | P-I-SD  | 81 | INP-GB  | 97  | MID-02  | 113 | CTR-8P  |
| 66 | P-I-MT  | 82 | INP-FA  | 98  | MID-03  |     |         |
| 67 | P-I-ML  | 83 | INP-FB  | 99  | MID-04  |     |         |
| 68 | P-I-MA  | 84 | INP-EA  | 100 | MID-05  |     |         |
| 69 | P-I-RS  | 85 | INP-EB  | 101 | MID-06  |     |         |
| 70 | P-I-RC  |    |         | 102 | MID-07  |     |         |
| 71 | P-I-CC  |    |         | 103 | MID-08  |     |         |
| 72 | P-I-HH  |    |         | 104 | MID-09  | 120 | CTR-RE  |
|    |         |    |         | 105 | MID-10  | 121 | CTR-GB  |
|    |         |    |         | 106 | MID-11  | 122 | CTR-EQ  |
|    |         |    |         | 107 | MID-12  | 123 | CTR-DX  |
|    |         |    |         | 108 | MID-13  |     |         |
|    |         |    |         | 109 | MID-14  |     |         |
|    |         |    |         | 110 | MID-15  |     |         |
|    |         |    |         | 111 | MID-16  |     |         |

If c = 1 (ROM/RAM machines):

| ID | Machine | ID | Machine | ID | Machine | ID | Machine |
| -- | ------- | -- | ------- | -- | ------- | -- | ------- |
| 00 | ROM-01  | 16 | ROM-17  | 32 | RAM-R1  | 48 | ROM-33  |
| 01 | ROM-02  | 17 | ROM-18  | 33 | RAM-R2  | 49 | ROM-34  |
| 02 | ROM-03  | 18 | ROM-19  | 34 | RAM-P1  | 50 | ROM-35  |
| 03 | ROM-04  | 19 | ROM-20  | 35 | RAM-P2  | 51 | ROM-36  |
| 04 | ROM-05  | 20 | ROM-21  |    |         | 52 | ROM-37  |
| 05 | ROM-06  | 21 | ROM-22  | 37 | RAM-R3  | 53 | ROM-38  |
| 06 | ROM-07  | 22 | ROM-23  | 38 | RAM-R4  | 54 | ROM-39  |
| 07 | ROM-08  | 23 | ROM-24  | 39 | RAM-P3  | 55 | ROM-40  |
| 08 | ROM-09  | 24 | ROM-25  | 40 | RAM-P4  | 56 | ROM-41  |
| 09 | ROM-10  | 25 | ROM-26  |    |         | 57 | ROM-42  |
| 10 | ROM-11  | 26 | ROM-27  |    |         | 58 | ROM-43  |
| 11 | ROM-12  | 27 | ROM-28  |    |         | 59 | ROM-44  |
| 12 | ROM-13  | 28 | ROM-29  |    |         | 60 | ROM-45  |
| 13 | ROM-14  | 29 | ROM-30  |    |         | 61 | ROM-46  |
| 14 | ROM-15  | 30 | ROM-31  |    |         | 62 | ROM-47  |
| 15 | ROM-16  | 31 | ROM-32  |    |         | 63 | ROM-48  |

### SYSEX set track routing

| MIDI Byte    | Purpose                          |
| ------------ | -------------------------------- |
| (SYSEX init) |                                  |
| `$5c`        | Set track routing ID             |
| `%0000aaaa`  | Route track `%aaaa` (0 to 15)... |
| `%00000bbb`  | ...to output specified by `%bbb` |
| `$f7`        | SYSEX end                        |

| Value | Output                   |
| ----- | ------------------------ |
| b:0   | OUTPUT A                 |
| b:1   | - B                      |
| b:2   | - C                      |
| b:3   | - D                      |
| b:4   | - E                      |
| b:5   | - F                      |
| b:6   | - MAIN (stereo pair A/B) |

### SYSEX set Rhythm Echo parameter

| MIDI Byte    | Purpose                            |
| ------------ | ---------------------------------- |
| (SYSEX init) |                                    |
| `$5d`        | Set delay tab parameter ID         |
| `%00000aaa`  | Target parameter `%aaa` (0 to 7)   |
| `%0bbbbbbb`  | Set value to `%bbbbbbb` (0 to 127) |
| `$f7`        | SYSEX end                          |

### SYSEX set Gate box parameter

| MIDI Byte    | Purpose                            |
| ------------ | ---------------------------------- |
| (SYSEX init) |                                    |
| `$5e`        | Set reverb parameter ID            |
| `%00000aaa`  | Target parameter `%aaa` (0 to 7)   |
| `%0bbbbbbb`  | Set value to `%bbbbbbb` (0 to 127) |
| `$f7`        | SYSEX end                          |

### SYSEX set the EQ parameter

| MIDI Byte    | Purpose                            |
| ------------ | ---------------------------------- |
| (SYSEX init) |                                    |
| `$5f`        | Set eq parameter ID                |
| `%00000aaa`  | Target parameter `%aaa` (0 to 7)   |
| `%0bbbbbbb`  | Set value to `%bbbbbbb` (0 to 127) |
| `$f7`        | SYSEX end                          |

### SYSEX set Dynamix parameter

| MIDI Byte    | Purpose                            |
| ------------ | ---------------------------------- |
| (SYSEX init) |                                    |
| `$60`        | Set dynamics parameter ID          |
| `%00000aaa`  | Target parameter `%aaa` (0 to 7)   |
| `%0bbbbbbb`  | Set value to `%bbbbbbb` (0 to 127) |
| `$f7`        | SYSEX end                          |

### SYSEX set tempo

| MIDI Byte    | Purpose      |
| ------------ | ------------ |
| (SYSEX init) |              |
| `$61`        | Set tempo ID |
| `%0aaaaaaa`  | Upper bits   |
| `%0bbbbbbb`  | Lower bits   |
| `$f7`        | SYSEX end    |

> [!NOTE]
> Tempo = `%aaaaaaabbbbbbb` / 24, max 300 BPM, min 30 BPM

### SYSEX set LFO

| MIDI Byte    | Purpose                                                     |
| ------------ | ----------------------------------------------------------- |
| (SYSEX init) |                                                             |
| `$62`        | Set LFO ID                                                  |
| `%0aaaabbb`  | `%aaaa`=LFO number (0 to 15), `%bbb`=parameter number (0-7) |
| `%0ccccccc`  | parameter value                                             |
| `$f7`        | SYSEX end                                                   |

> [!NOTE]
> You can also use control change to set LFO; speed, amount & shapemix.

### SYSEX unused (`$63`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$63`        | None      |
| `$f7`        | SYSEX end |

### SYSEX reset midi note map

| MIDI Byte    | Purpose                                   |
| ------------ | ----------------------------------------- |
| (SYSEX init) |                                           |
| `$64`        | Reset midi note map to Elektron's mapping |
| `$f7`        | SYSEX end                                 |

### SYSEX set trig group

| MIDI Byte    | Purpose                                 |
| ------------ | --------------------------------------- |
| (SYSEX init) |                                         |
| `$65`        | Set trig group ID                       |
| `%0000aaaa`  | Trigger track `%aaaa` (0 to 15)         |
| `%0000bbbb`  | track `%bbbb` (0 to 15) to be triggered |
| `$f7`        | SYSEX end                               |

### SYSEX set mute group

| MIDI Byte    | Purpose                              |
| ------------ | ------------------------------------ |
| (SYSEX init) |                                      |
| `$66`        | Set mute group ID                    |
| `%0000aaaa`  | Mute control track `%aaaa` (0 to 15) |
| `%0000bbbb`  | track `%bbbb` (0 to 15) to be muted  |
| `$f7`        | SYSEX end                            |

### SYSEX pattern dump

| MIDI Byte    | Purpose                                         |
| ------------ | ----------------------------------------------- |
| (SYSEX init) |                                                 |
| `$67`        | Pattern dump ID                                 |
| ...          | Pattern data bytes (see separate documentation) |
| `$f7`        | SYSEX end                                       |

### SYSEX pattern dump request

| MIDI Byte    | Purpose                 |
| ------------ | ----------------------- |
| (SYSEX init) |                         |
| `$68`        | Pattern request ID      |
| `%0aaaaaaa`  | Send pattern `%aaaaaaa` |
| `$f7`        | SYSEX end               |

### SYSEX song dump

| MIDI Byte    | Purpose                                      |
| ------------ | -------------------------------------------- |
| (SYSEX init) |                                              |
| `$69`        | Song dump ID                                 |
| ...          | Song data bytes (see separate documentation) |
| `$f7`        | SYSEX end                                    |

### SYSEX song request

| MIDI Byte    | Purpose                      |
| ------------ | ---------------------------- |
| (SYSEX init) |                              |
| `$6a`        | Song request ID              |
| `%000aaaaa`  | send song `%aaaaa` (0 to 31) |
| `$f7`        | SYSEX end                    |

### SYSEX set receive position for dump

| MIDI Byte    | Purpose                                                         |
| ------------ | --------------------------------------------------------------- |
| (SYSEX init) |                                                                 |
| `$6b`        | Receive position ID                                             |
| `%000aaaab`  | receive pos on type 0001=global 0010=kit 0100=pattern 1000=song |
| `%0ccccccc`  | to b=0 => pos ccccccc, b=1 original position                    |
| `%0ddddddd`  | for ddddddd sysexes of this type                                |
| `$f7`        | SYSEX end                                                       |

> [!NOTE]
> Same as setting the receive parameters in the SYSEX menu.

### SYSEX load song

| MIDI Byte    | Purpose                      |
| ------------ | ---------------------------- |
| (SYSEX init) |                              |
| `$6c`        | Load song ID                 |
| `%000aaaaa`  | Load song `%aaaaa` (0 to 31) |
| `$f7`        | SYSEX end                    |

### SYSEX save song

| MIDI Byte    | Purpose                                  |
| ------------ | ---------------------------------------- |
| (SYSEX init) |                                          |
| `$6d`        | Save song ID                             |
| `%000aaaaa`  | Save song to position `%aaaaa` (0 to 31) |
| `$f7`        | SYSEX end                                |

### SYSEX status request

| MIDI Byte    | Purpose                                         |
| ------------ | ----------------------------------------------- |
| (SYSEX init) |                                                 |
| `$70`        | Request status ID                               |
| `%00aaaaaa`  | Status for parameter `%aaaaaa` (see list below) |
| `$f7`        | SYSEX end                                       |

| Value  | Parameter                                            |
| ------ | ---------------------------------------------------- |
| `0x01` | Current global slot (0 to 7)                         |
| `0x02` | Current kit number (0 to 63)                         |
| `0x04` | Current pattern number (A1=0, A2=1, ... B1=16, ...)  |
| `0x08` | Current song number (0 to 31)                        |
| `0x10` | Current sequencer mode (pattern mode=0, song mode=1) |
| `0x20` | Current lock mode (classic=0, extended=1)            |

### SYSEX set status

| MIDI Byte    | Purpose                        |
| ------------ | ------------------------------ |
| (SYSEX init) |                                |
| `$71`        | Set status ID                  |
| `%00aaaaaa`  | Set parameter `%aaaaaa` to ... |
| `%0bbbbbbb`  | ... value `%bbbbbbb`           |
| `$f7`        | SYSEX end                      |

| Value  | Parameter                                            |
| ------ | ---------------------------------------------------- |
| `0x01` | Current global slot (0 to 7)                         |
| `0x02` | Current kit number (0 to 63)                         |
| `0x04` | Current pattern number (A1=0, A2=1, ... B1=16, ...)  |
| `0x08` | Current song number (0 to 31)                        |
| `0x10` | Current sequencer mode (pattern mode=0, song mode=1) |
| `0x20` | Current lock mode (classic=0, extended=1)            |
| `0x22` | Current track (0 to 15)                              |

### SYSEX status response

| MIDI Byte    | Purpose                               |
| ------------ | ------------------------------------- |
| (SYSEX init) |                                       |
| `$72`        | Status response ID                    |
| `%00aaaaaa`  | Parameter `%aaaaaa` has ...           |
| `%0bbbbbbb`  | ... value `%bbbbbbb` (see list below) |
| `$f7`        | SYSEX end                             |

| Value  | Parameter                                            |
| ------ | ---------------------------------------------------- |
| `0x01` | Current global slot (0 to 7)                         |
| `0x02` | Current kit number (0 to 63)                         |
| `0x04` | Current pattern number (A1=0, A2=1, ... B1=16, ...)  |
| `0x08` | Current song number (0 to 31)                        |
| `0x10` | Current sequencer mode (pattern mode=0, song mode=1) |
| `0x20` | Current lock mode (classic=0, extended=1)            |
| `0x22` | Current track (0 to 15)                              |

### SYSEX set UW sample name

| MIDI Byte    | Purpose                                   |
| ------------ | ----------------------------------------- |
| (SYSEX init) |                                           |
| `$73`        | Set sample name ID                        |
| `%00aaaaaa`  | Sample number (0-47 on mkII, 0-31 on mkI) |
| ...          | 4 bytes ASCII name (7-bit)                |
| `$f7`        | SYSEX end                                 |

> [!NOTE]
> This affects samples in the current samplebank only.

> [!NOTE]
> During sample dumps, sysex data can be sent in between the sample dump header and the sample data. In this scenario, the sample number is ignored and is instead read from the sample dump header.

## 2. TurboMIDI Protocol SysEx

All TurboMIDI SYSEX messages start with this sequence:

| MIDI Byte | Purpose                     |
| --------- | --------------------------- |
| `$f0`     | SYSEX Start                 |
| `$00`     | Europe/USA ID               |
| `$20`     | Europe ID                   |
| `$3c`     | Elektron ESI ID             |
| `$00`     | Generic Elektron product ID |
| `$00`     | Base channel (Padding)      |

A complete SYSEX message looks like this:

`$f0,$00,$20,$3c,$00,$00,command,...,$f7`

### SYSEX turbomidi speed request

| MIDI Byte    | Purpose          |
| ------------ | ---------------- |
| (SYSEX init) |                  |
| `$10`        | Speed request ID |
| `$f7`        | SYSEX end        |

### SYSEX turbomidi speed answer

| MIDI Byte    | Purpose                                                   |
| ------------ | --------------------------------------------------------- |
| (SYSEX init) |                                                           |
| `$11`        | Speed answer ID                                           |
| `%aaaaaaaa`  | bit 0 = speed 2, bit 1, = speed 3.3...                    |
| `%00000bbb`  | bit 0 = speed 13.3x, bit 1 = speed 16x, bit 2 = speed 20x |
| `%cccccccc`  | contains certified speed bits                             |
| `%00000bbb`  | contains certified speed bits                             |
| `$f7`        | SYSEX end                                                 |

### SYSEX turbomidi speed negotiation (master)

| MIDI Byte    | Purpose             |
| ------------ | ------------------- |
| (SYSEX init) |                     |
| `$12`        | Speed negotiaton ID |
| `%0000aaaa`  | Speed 1 value       |
| `%0000bbbb`  | Speed 2 value       |
| `$f7`        | SYSEX end           |

SPEED1 >= SPEED2 (= if certified)
SPEED1, SPEED2 = 1 requires no handshaking

### SYSEX turbomidi speed acknowledgement (slave)

| MIDI Byte    | Purpose                 |
| ------------ | ----------------------- |
| (SYSEX init) |                         |
| `$13`        | Speed acknowledgment ID |
| `$f7`        | SYSEX end               |

Acknowledge sent upon SPEEDNEG, always sent at 1xMIDI_SPEED. After speed acknowledgement is received SPEED1 is set on both the master and slave device.

### SYSEX turbomidi speed test (master)

| MIDI Byte    | Purpose                |
| ------------ | ---------------------- |
| (SYSEX init) |                        |
| `$14`        | Speed test (master) ID |
| `$55`        |                        |
| `$55`        |                        |
| `$55`        |                        |
| `$55`        |                        |
| `$00`        |                        |
| `$00`        |                        |
| `$00`        |                        |
| `$00`        |                        |
| `$f7`        | SYSEX end              |

Before the speedtest the master should allow the slave some breathing by sending 16 `0x00` bytes to allow setting of speed and resetting of UART.

### SYSEX turbomidi speed result (slave)

| MIDI Byte    | Purpose                |
| ------------ | ---------------------- |
| (SYSEX init) |                        |
| `$15`        | Speed test (result) ID |
| `$55`        |                        |
| `$55`        |                        |
| `$55`        |                        |
| `$55`        |                        |
| `$00`        |                        |
| `$00`        |                        |
| `$00`        |                        |
| `$00`        |                        |
| `$f7`        | SYSEX end              |

### SYSEX turboMIDI speed test 2 (master)

| MIDI Byte    | Purpose                  |
| ------------ | ------------------------ |
| (SYSEX init) |                          |
| `$16`        | Speed test 2 (master) ID |
| `$f7`        | SYSEX end                |

### SYSEX turboMIDI speed result 2 (slave)

| MIDI Byte    | Purpose                  |
| ------------ | ------------------------ |
| (SYSEX init) |                          |
| `$17`        | Speed test 2 (result) ID |
| `$f7`        | SYSEX end                |

After reception of "speed result 2" SPEED2 is set on both the master and slave device.

### Possible Speed and Max Speed parameters

| SPEED | x 31.25 kbit |
| ----- | ------------ |
| 1     | 1            |
| 2     | 2            |
| 3     | 3.33         |
| 4     | 4            |
| 5     | 5            |
| 6     | 6.66         |
| 7     | 8            |
| 8     | 10           |
| (9)   | 13.3         |
| (10)  | 16           |
| (11)  | 20           |

Values within parentheses are not supported.

### Active sensing

Active sensing should be sent from any device supporting Turbo MIDI whenever a higher speed is used (TM-1 approx 150ms between). If a Turbo MIDI device receives active sensing and then active sensing is lost for more than 300ms the device will revert to normal MIDI speed.

### Negotiation Timing

Length of master timeouts: 30 ms minimum. Length of slave timeouts: 15ms minimum, 25 ms maximum. The timeout is for a byte, not a message; a message could take 15xN ms to reach the slave.
