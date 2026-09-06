# Analog Drive MIDI implementation

`Analog-Drive-User-Manual_1.2.pdf`: Page 19

---

This appendix lists the MIDI specifications for the Analog Drive.

- **Program change:** 0–99.
- **CV-mode:** 10 bits, 7 in MSB, 3 in LSB. MSB=64 equals 0 V.
- **Expression pedal mode:** 9 bits. 7 in MSB, 2 in LSB.
- **MIDI CC:** 8 bits. 7 in MSB, 1 in LSB.

## 1. MIDI CC

| Parameter       | CC MSB | CC LSB | Values |
| --------------- | ------ | ------ | ------ |
| Gain            | 16     | 48     | -      |
| Low             | 17     | 49     | -      |
| Mid Freq        | 18     | 50     | -      |
| Mid             | 19     | 51     | -      |
| High            | 20     | 52     | -      |
| Level           | 21     | 53     | -      |
| Expression Gain | 4      | 36     | -      |
| Expression Mid  | 1      | 33     | -      |
| Circuit Select  | 3      | -      | 0–15 = Clean Boost<br>16–31 = Mid Drive<br>32–47 = Dirty Drive<br>48–63 = Big Dist<br>64–79 = Focused Dist<br>80–95 = Harmonic Fuzz<br>96–111 = High Gain<br>112–127 = Thick Gain |
| Effect Active   | 80     | -      | 1–63 = Off<br>64–127 = On |
