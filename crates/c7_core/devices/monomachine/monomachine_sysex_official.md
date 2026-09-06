# Monomachine SysEx implementation

`monomachine_manual_OS1.32.pdf`: Pages 147 - 151

---

This appendix lists all Monomachine SYSEX messages available for external control.

Data printed with a `$` sign is written in hexadecimal format.
Data printed with a `%` sign is written in a binary bitfield format.

## 1. Monomachine SysEx Messages

All SYSEX messages start with this sequence.

| MIDI Byte | Purpose                        |
| --------- | ------------------------------ |
| `$f0`     | SYSEX Start                    |
| `$00`     | Europe/USA ID                  |
| `$20`     | Europe ID                      |
| `$3c`     | Elektron Music Machines MAV ID |
| `$03`     | Monomachine ID                 |
| `$00`     | Base channel (Padding)         |

A complete SYSEX message looks like this:

`$f0,$00,$20,$3c,$03,$00,command,...,$f7`

### SYSEX global settings dump

| MIDI Byte    | Purpose                   |
| ------------ | ------------------------- |
| (SYSEX init) |                           |
| `$50`        | Global settings dump ID   |
| ...          | Global setting data bytes |
| `$f7`        | SYSEX end                 |

### SYSEX global setting dump request

| MIDI Byte    | Purpose                                      |
| ------------ | -------------------------------------------- |
| (SYSEX init) |                                              |
| `$51`        | Global setting dump request ID               |
| `%00000aaa`  | Send global setting `%aaa` (0 to 7) by sysex |
| `$f7`        | SYSEX end                                    |

### SYSEX kit dump

| MIDI Byte    | Purpose        |
| ------------ | -------------- |
| (SYSEX init) |                |
| `$52`        | Kit dump ID    |
| ...          | Kit data bytes |
| `$f7`        | SYSEX end      |

### SYSEX kit request

| MIDI Byte    | Purpose                                       |
| ------------ | --------------------------------------------- |
| (SYSEX init) |                                               |
| `$53`        | Kit dump request ID                           |
| `%0aaaaaaa`  | Send kit number `%aaaaaa` (0 to 127) by sysex |
| `$f7`        | SYSEX end                                     |

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
| ...          | 11 bytes ASCII (7-bit)  |
| `$f7`        | SYSEX end               |

### SYSEX set active global setting

| MIDI Byte    | Purpose                                  |
| ------------ | ---------------------------------------- |
| (SYSEX init) |                                          |
| `$56`        | Set active global setting ID             |
| `%00000aaa`  | Set global pos `%aaa` (0 to 7) as active |
| `$f7`        | SYSEX end                                |

### SYSEX load pattern

| MIDI Byte    | Purpose                            |
| ------------ | ---------------------------------- |
| (SYSEX init) |                                    |
| `$57`        | Load pattern ID                    |
| `%0aaaaaaa`  | Load pattern `%aaaaaaa` (0 to 127) |
| `$f7`        | SYSEX end                          |

### SYSEX load kit

| MIDI Byte    | Purpose                        |
| ------------ | ------------------------------ |
| (SYSEX init) |                                |
| `$58`        | Load kit ID                    |
| `%0aaaaaaa`  | Load kit `%aaaaaaa` (0 to 127) |
| `$f7`        | SYSEX end                      |

### SYSEX save kit

| MIDI Byte    | Purpose                               |
| ------------ | ------------------------------------- |
| (SYSEX init) |                                       |
| `$59`        | Save kit ID                           |
| `%0aaaaaaa`  | Save kit to pos `%aaaaaaa` (0 to 127) |
| `$f7`        | SYSEX end                             |

### SYSEX unused (`$5a`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$5a`        | None      |
| `$f7`        | SYSEX end |

### SYSEX assign machine

| MIDI Byte    | Purpose                                                                                             |
| ------------ | --------------------------------------------------------------------------------------------------- |
| (SYSEX init) |                                                                                                     |
| `$5b`        | Load machine ID                                                                                     |
| `%00000aaa`  | Select track `%aaa` (0 to 5)...                                                                     |
| `%0bbbbbbb`  | ...and assign machine `%bbbbbbb` number (see list below)                                            |
| `%000000dd`  | 0 = do not init data pages / 1 = init all data pages / 2 = init synthesis page (optional parameter) |
| `$f7`        | SYSEX end                                                                                           |

| ID | Machine    |
| -- | ---------- |
| 00 | GND-GND    |
| 01 | GND-SIN    |
| 02 | GND-NOIS   |
| 03 | SID-6581   |
| 04 | SWAVE-SAW  |
| 05 | SWAVE-PULS |
| 06 | DPRO-WAVE  |
| 07 | DPRO-BBOX  |
| 08 | FM+-STAT   |
| 09 | FM+-PAR    |
| 10 | FM+-DYN    |
| 11 | VO-VO-6    |
| 12 | FX-THRU    |
| 13 | FX-REVERB  |
| 14 | SWAVE-ENS  |
| 15 | FX-CHORUS  |
| 16 | FX-DYNAMIX |
| 17 | FX-RINGMOD |
| 18 | FX-PHASER  |
| 19 | FX-FLANGER |
| 32 | DPRO-DDRW  |
| 33 | DPRO-DENS  |

### SYSEX set track routing

| MIDI Byte    | Purpose                                              |
| ------------ | ---------------------------------------------------- |
| (SYSEX init) |                                                      |
| `$5c`        | Set track routing ID                                 |
| `%00000aaa`  | Route track `%aaa` (0 to 5)...                       |
| `%00000bcd`  | ...to output bus specified by `%bcd` ...             |
| `%00000eee`  | with inputs specified by `%eee` (optional parameter) |
| `$f7`        | SYSEX end                                            |

| Value | Mapping  |
| ----- | -------- |
| b     | BUS EF   |
| c     | BUS CD   |
| d     | BUS AB   |
| e:0   | NEIGHBOR |
| e:1   | INP A    |
| e:2   | INP B    |
| e:3   | INP A+B  |
| e:4   | BUS AB   |
| e:5   | BUS CD   |
| e:6   | BUS EF   |

### SYSEX digipro waveform dump

| MIDI Byte    | Purpose                     |
| ------------ | --------------------------- |
| (SYSEX init) |                             |
| `$5d`        | Digipro waveform dump ID    |
| ...          | Digipro waveform data bytes |
| `$f7`        | SYSEX end                   |

### SYSEX digipro waveform request

| MIDI Byte    | Purpose                                  |
| ------------ | ---------------------------------------- |
| (SYSEX init) |                                          |
| `$5e`        | Digipro waveform request ID              |
| `%00aaaaaa`  | Send waveform number `%aaaaaa` (0 to 63) |
| `$f7`        | SYSEX end                                |

### SYSEX set Gate box parameter

| MIDI Byte    | Purpose                            |
| ------------ | ---------------------------------- |
| (SYSEX init) |                                    |
| `$5e`        | Set reverb parameter ID            |
| `%00000aaa`  | Target parameter `%aaa` (0 to 7)   |
| `%0bbbbbbb`  | Set value to `%bbbbbbb` (0 to 127) |
| `$f7`        | SYSEX end                          |

### SYSEX unused (`$5f`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$5f`        | None      |
| `$f7`        | SYSEX end |

### SYSEX unused (`$60`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$60`        | None      |
| `$f7`        | SYSEX end |

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

### SYSEX unused (`$62`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$62`        | None      |
| `$f7`        | SYSEX end |

### SYSEX unused (`$63`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$63`        | None      |
| `$f7`        | SYSEX end |

### SYSEX unused (`$64`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$64`        | None      |
| `$f7`        | SYSEX end |

### SYSEX unused (`$65`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$65`        | None      |
| `$f7`        | SYSEX end |

### SYSEX unused (`$66`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$66`        | None      |
| `$f7`        | SYSEX end |

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
| `%000aaaaa`  | send song `%aaaaa` (0 to 23) |
| `$f7`        | SYSEX end                    |

### SYSEX unused (`$6b`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$6b`        | None      |
| `$f7`        | SYSEX end |

### SYSEX load song

| MIDI Byte    | Purpose                      |
| ------------ | ---------------------------- |
| (SYSEX init) |                              |
| `$6c`        | Load song ID                 |
| `%000aaaaa`  | Load song `%aaaaa` (0 to 23) |
| `$f7`        | SYSEX end                    |

### SYSEX save song

| MIDI Byte    | Purpose                                  |
| ------------ | ---------------------------------------- |
| (SYSEX init) |                                          |
| `$6d`        | Save song ID                             |
| `%000aaaaa`  | Save song to position `%aaaaa` (0 to 23) |
| `$f7`        | SYSEX end                                |

### SYSEX unused (`$6e`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$6e`        | None      |
| `$f7`        | SYSEX end |

### SYSEX unused (`$6f`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$6f`        | None      |
| `$f7`        | SYSEX end |

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
| `0x02` | Current kit number (0 to 127)                        |
| `0x04` | Current pattern number (A1=0, A2=1, ... B1=16, ...)  |
| `0x08` | Current song number (0 to 23)                        |
| `0x10` | Current sequencer mode (pattern mode=0, song mode=1) |
| `0x20` | Current audio mode (mono=0, poly=1)                  |
| `0x21` | Current sequencer mode mode (audio=0, midi=1)        |
| `0x22` | Current audio track (0 to 5)                         |
| `0x23` | Current midi seq track (0 to 5)                      |

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
| `0x02` | Current kit number (0 to 127)                        |
| `0x04` | Current pattern number (A1=0, A2=1, ... B1=16, ...)  |
| `0x08` | Current song number (0 to 23)                        |
| `0x10` | Current sequencer mode (pattern mode=0, song mode=1) |
| `0x20` | Current audio mode (mono=0, poly=1)                  |
| `0x21` | Current sequencer mode mode (audio=0, midi=1)        |

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
| `0x02` | Current kit number (0 to 127)                        |
| `0x04` | Current pattern number (A1=0, A2=1, ... B1=16, ...)  |
| `0x08` | Current song number (0 to 23)                        |
| `0x10` | Current sequencer mode (pattern mode=0, song mode=1) |
| `0x20` | Current audio mode (mono=0, poly=1)                  |
| `0x21` | Current sequencer mode mode (audio=0, midi=1)        |
| `0x22` | Current audio track (0 to 5)                         |
| `0x23` | Current midi seq track (0 to 5)                      |

### SYSEX unused (`$73`)

| MIDI Byte    | Purpose   |
| ------------ | --------- |
| (SYSEX init) |           |
| `$73`        | None      |
| `$f7`        | SYSEX end |

## 2. TurboMIDI Protocol SysEx

All TurboMIDI SYSEX messages start with this sequence:

| MIDI Byte | Purpose                        |
| --------- | ------------------------------ |
| `$f0`     | SYSEX Start                    |
| `$00`     | Europe/USA ID                  |
| `$20`     | Europe ID                      |
| `$3c`     | Elektron Music Machines MAV ID |
| `$00`     | Generic Elektron product ID    |
| `$00`     | Base channel (Padding)         |

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