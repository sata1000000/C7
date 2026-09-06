Configuration and protocol documentation for the Elektron Analog Four.

> [!WARNING]
> **Analog Four SysEx support is not implemented.**
>
> The device JSON is enabled, so the Analog Four is active in the codebase. Everything that uses MIDI works. Almost nothing that uses SysEx is currently working.
>
> A major restructure will have to take place if C7 is going to integrate the Analog Four.
> This is because the Analog Four handles its SysEx stream slightly differently from typical Elektron devices.
>
> | Section    | MnM                 | A4                  |
> |------------|---------------------|---------------------|
> | Header     | `F0 00 20 3C 03 00` | `F0 00 20 3C 06 00` |
> | Cmd        | `[CMD]`             | `[CMD]`             |
> | Proto      | **None**            | `01 01`             |
> | Data       | `[DATA]`            | `[DATA]`            |
> | Pre-footer | **None**            | `00 00 00 05`       |
> | End        | `F7`                | `F7`                |
>
> There'd need to be additional 'Proto' and 'Pre-footer' sections in every SysEx call in the project.