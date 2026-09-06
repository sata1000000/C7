Each supported device has its own directory containing a JSON config file and related documentation.

Every device JSON assumes the device is the latest model and is running the latest community firmware.

If C7 is going to be a full C6 replacement, C7 will have to eventually support all the same devices that C6 supported.

Modern Elektron devices might get support later. C6 parity is the first priority.

| Device                                   | USB    | MIDI            | C7 as C6 Parity | C6 Features                                                       |
| ---------------------------------------- | ------ | --------------- | --------------- | ----------------------------------------------------------------- |
| [SidStation][sidstation]                 | No     | 5-pin DIN       | Partial         | OS updates, SysEx patch management                                |
| [Machinedrum SPS-1 MKI][machinedrum]     | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management                                |
| [Monomachine SFX-6][monomachine]         | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management                                |
| [Monomachine SFX-60 MKI][monomachine]    | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management                                |
| [Machinedrum SPS-1 MKII][machinedrum]    | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management                                |
| [Machinedrum SPS-1UW MKI][machinedrum]   | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management, Sample transfer (SDS)         |
| TM-1                                     | Type B | MIDI over USB   | Passthrough     | OS updates                                                        |
| [Monomachine SFX-60 MKII][monomachine]   | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management                                |
| [Machinedrum SPS-1UW MKII][machinedrum]  | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management, Sample transfer (SDS)         |
| [Machinedrum SPS-1+ MKII][machinedrum]   | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management                                |
| [Machinedrum SPS-1UW+ MKII][machinedrum] | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management, Sample transfer (SDS)         |
| [Monomachine SFX-60+ MKII][monomachine]  | No     | 5-pin DIN       | Full            | OS updates, SysEx patch management, DigiPro transfer              |
| [Octatrack DPS-1 MKI][octatrack]         | Type B | 5-pin DIN       | Full            | OS updates                                                        |
| [Analog Four MKI][analog-four]           | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management                                |
| Analog Keys                              | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management                                |
| [Analog Rytm MKI][analog-rytm]           | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management, Sample transfer (SDS)         |
| [Analog Drive][analog-drive]             | No     | 5-pin DIN       | None            | OS updates, SysEx patch management                                |
| Analog Heat MKI                          | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management                                |
| Digitakt MKI                             | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management, Sample transfer (SDS)         |
| [Analog Four MKII][analog-four]          | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management                                |
| [Analog Rytm MKII][analog-rytm]          | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management, Sample transfer (SDS)         |
| [Octatrack DPS-1 MKII][octatrack]        | Type B | 5-pin DIN       | Full            | OS updates                                                        |
| Analog Heat MKII                         | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management                                |
| Digitone MKI                             | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management                                |
| Digitone Keys                            | Type B | 5-pin DIN + USB | None            | OS updates, SysEx patch management                                |

<!-- Device directories -->
[sidstation]: sidstation
[machinedrum]: machinedrum
[monomachine]: monomachine
[octatrack]: octatrack
[analog-four]: analog-four
[analog-rytm]: analog-rytm
[analog-drive]: analog-drive
