# The C7 File Format

The `.c7` file format is the storage format used by C7 for backing up Elektron hardware data such as kits, patterns, songs, and global settings.

## Basic Structure

Every `.c7` file is a JSON object whose keys are section names. The `"c7"` key holds file-level metadata, while the remaining keys are the data sections.

```json
{
  "c7": {
    "type": "kit",
    "device": "Machinedrum",
    "export_time": "2026-04-21T16:30:00Z",
    "description": "Machine assignments"
  },
  "kit_5": {
    "name": "TECHNOKIT",
    "type": "kit",
    "format": "sysex",
    "data": "f000203c02005200...f7"
  }
}
```

A data section's key is its `type`, an underscore, and its 1-based slot number, so `kit_5` is the fifth kit.

> [!NOTE]
> A single-file export must still keep the slot number in the section key (e.g. `"kit_5"` instead of just `"kit"`).

### `"c7"` Header Keys

| Key           | Required | Description |
|---------------|----------|-------------|
| `name`        | No       | Display name for the file. <br>Written only for `digipro_wavetable`, where it names the whole wavetable. |
| `type`        | Yes      | What the file holds: `kit`, `pattern`, `song`, `global`, `sample`, `digipro`, `digipro_wavetable`, `sound`, or `bundle`. |
| `device`      | No       | Source device (e.g. `Machinedrum`). |
| `export_time` | No       | When the data was exported. <br>Always UTC to the second, written as `YYYY-MM-DDThh:mm:ssZ` ([ISO 8601](https://en.wikipedia.org/wiki/ISO_8601)). |
| `description` | No       | Short summary of what the file holds. <br>Defaults from `type`. |

`bundle` is a header type only. It describes the file as a whole and never appears as a data section's `type`.

### Data Section Keys

| Key          | Required | Description |
|--------------|----------|-------------|
| `name`       | No       | Display name for the item. <br>Doubles as the suggested filename. |
| `type`       | Yes      | What the section holds: `kit`, `pattern`, `song`, `global`, `sample`, `digipro`, or `sound`. |
| `sound_type` | No       | What kind of sound it is (e.g. `"BD Bass Drum"`, `"Pad"`, `"Bass"`). <br>Omitted when the user leaves the type blank. |
| `format`     | Yes      | How `data` is encoded. <br>`sysex` for SysEx payloads, `json` for parameter objects, or `flac+base64` for audio. |
| `comment`    | No       | Text note left by the user describing the item. |
| `data`       | Yes      | The payload. <br>A [hex](https://en.wikipedia.org/wiki/Hexadecimal) string for `sysex`, an array of hex strings when the payload holds more than one SysEx message, an object for `json`, or a Base64 string for `flac+base64`. |

The `name` field is the display name for the item, and it doubles as the suggested filename. Kits, songs, and DigiPro frames embed a name in their SysEx, and `name` copies it, so those names use only the uppercase characters the device allows. The embedded name stays authoritative, and `name` is only a readable copy of it. Sounds have no embedded name, so theirs is whatever the user typed. Patterns and globals have no name at all, neither embedded in the SysEx nor written into the file.

## Storing `bundle` Data (`"type": "bundle"`)

A `bundle` is a `.c7` file containing multiple types of items.

```json
{
  "c7": {
    "type": "bundle",
    "device": "Machinedrum",
    "export_time": "2026-04-21T16:30:00Z",
    "description": "Bundle containing multiple data types"
  },
  "global_1": {
    "type": "global",
    "format": "sysex",
    "data": "f000203c02005000...f7"
  },
  "kit_1": {
    "name": "TECHNOKIT",
    "type": "kit",
    "format": "sysex",
    "data": "f000203c02005200...f7"
  },
  "pattern_1": {
    "type": "pattern",
    "format": "sysex",
    "data": "f000203c02006700...f7"
  },
  "song_1": {
    "name": "UW DEMO",
    "type": "song",
    "format": "sysex",
    "data": "f000203c02006900...f7"
  },
  "sample_1": {
    "name": "KICK",
    "type": "sample",
    "format": "flac+base64",
    "data": "ZkxhQwAAACIQABAAAAfSABWkCsRAsAAAfsA/ihxNkOK3YVrIA075LWG4hAAAQQIA..."
  }
}
```

## Storing `sample` Data (`"type": "sample"`)

A `sample` section holds a single audio sample, embedded directly into the file.

The SDS packets are decoded and re-encoded at the bit depth the dump itself declared. The data is encoded in two stages:

1. **Lossless Compression:** Raw sample data (SDS) is encoded losslessly as [FLAC](https://en.wikipedia.org/wiki/FLAC). The SDS Dump Header is copied into an `SDS_DUMP_HEADER` [Vorbis comment tag](https://en.wikipedia.org/wiki/Vorbis_comment) as a hex string, since FLAC has no slot for a sample period, loop points, or a sample number.
2. **Base64 Encoding:** The compressed FLAC data is then encoded into a [Base64](https://en.wikipedia.org/wiki/Base64) string.

Because FLAC encoders only accept standard bit depths (like 16 or 24-bit), the compressed FLAC file might have a higher bit depth than the original sample. On import, the FLAC stream's bit depth is ignored, and the audio is repacked using the true bit depth stored in the Vorbis comment tag.

```json
{
  "c7": {
    "type": "sample",
    "device": "Machinedrum",
    "export_time": "2026-04-21T16:30:00Z",
    "description": "Audio sample data"
  },
  "sample_1": {
    "name": "KICK",
    "type": "sample",
    "format": "flac+base64",
    "data": "ZkxhQwAAACIQABAAAAfSABWkCsRAsAAAfsA/ihxNkOK3YVrIA075LWG4hAAAQQIA..."
  }
}
```

## Storing `sound` Data (`"type": "sound"`)

A `sound` section holds a single sound extracted from a kit's track.

### Storing as SysEx (`"format": "sysex"`)

Modern devices, like the Analog Four, have a native SysEx command for loading a single sound.

```json
{
  "c7": {
    "type": "sound",
    "device": "Analog Four",
    "export_time": "2026-04-21T16:30:00Z"
  },
  "sound_5": {
    "name": "Nice Kick",
    "type": "sound",
    "sound_type": "BD Bass Drum",
    "format": "sysex",
    "data": "f000203c02005200...f7"
  }
}
```

### Storing as JSON (`"format": "json"`)

Older devices, like the Machinedrum and Monomachine, have no native SysEx command for loading a single sound. The sound data is stored as a JSON object with named parameters that are keyed by page.

On import, the track blob is reconstructed, spliced into a kit fetched from the device, and written back to the device.

```json
{
  "c7": {
    "type": "sound",
    "device": "Machinedrum",
    "export_time": "2026-04-26T12:00:00Z",
    "description": "Individual sound data extracted from a kit"
  },
  "sound_3": {
    "name": "Nice Kick",
    "type": "sound",
    "sound_type": "BD Bass Drum",
    "format": "json",
    "comment": "This Kick is nice",
    "data": {
      "machine": "TRX-BD",
      "pages": {
        "standalone": {
          "Level": {
            "LEV": 100
          }
        },
        "synth": {
          "Synthesis": {
            "CC 1": 64,
            "CC 2": 80,
            "CC 3": 40,
            "CC 4": 100,
            "CC 5": 0,
            "CC 6": 0,
            "CC 7": 0,
            "CC 8": 64
          },
          "Effects": {
            "AMD": 0,
            "AMF": 0,
            "EQF": 64,
            "EQG": 64,
            "FLTF": 0,
            "FLTW": 127,
            "FLTQ": 0,
            "SRR": 0
          },
          "Routing": {
            "DIST": 0,
            "VOL": 127,
            "PAN": 64,
            "DEL": 0,
            "REV": 0,
            "LFOS": 64,
            "LFOD": 0,
            "LFOM": 0
          }
        }
      },
      "track_lfo": {
        "LFO": {
          "TRACK": 0,
          "PARAM": 0,
          "SHP1": 0,
          "SHP2": 0,
          "UPDTE": 0
        }
      }
    }
  }
}
```

Sound section names follow `sound_N`, where `N` is the track slot (e.g. `sound_1`–`sound_16` for Machinedrum, `sound_1`–`sound_6` for Monomachine).

Page keys use the `fullname` field from the device spec (`Synthesis`, `Effects`, `Routing`). Param keys use the `name` field from the device spec. The Synthesis page uses generic CC labels (`CC 1`…`CC 8`), since what those knobs do depends on the machine assigned to the track. The Effects and Routing pages use named params (`EQF`, `FLTW`, `VOL`, `PAN`, and so on), which mean the same thing on every machine.

LFO is limited to the five SysEx-controlled parameters (`TRACK`, `PARAM`, `SHP1`, `SHP2`, `UPDTE`). The three speed/depth/mix values (`LFOS`, `LFOD`, `LFOM`) are already captured in the Routing page. The 31-byte internal LFO oscillator state is not exposed via the public API and is zeroed on reconstruction.

## Storing `digipro` Data (`"type": "digipro"` or `"digipro_wavetable"`)

The Monomachine's DigiPro engine accepts individual waveform frames through SysEx. Two `.c7` header types cover this data.

### Storing as a Single Frame (`"type": "digipro"`)

A single DigiPro waveform frame.

```json
{
  "c7": {
    "type": "digipro",
    "device": "Monomachine",
    "export_time": "2026-05-01T12:00:00Z",
    "description": "Single DigiPro waveform"
  },
  "digipro_1": {
    "name": "WAVE",
    "type": "digipro",
    "format": "sysex",
    "data": "f000203c03005d01010057415645...f7"
  }
}
```

`data` is the DigiPro SysEx as a hex string.

### Storing as Multiple Frames (`"type": "digipro_wavetable"`)

A sequence of DigiPro waveform frames.

Unlike other data types, the original slot number is not kept. Frames are numbered from `digipro_1` up to `digipro_64`.

```json
{
  "c7": {
    "name": "WAVE",
    "type": "digipro_wavetable",
    "device": "Monomachine",
    "export_time": "2026-05-01T12:00:00Z",
    "description": "DigiPro wavetable (Sequence of DigiPro frames)"
  },
  "digipro_1": {
    "name": "WAVE",
    "type": "digipro",
    "format": "sysex",
    "data": "f000203c03005d010100...f7"
  },
  "digipro_2": {
    "name": "WAVE",
    "type": "digipro",
    "format": "sysex",
    "data": "f000203c03005d010101...f7"
  }
}
```

## Future-Proofing

The `.c7` format is built for archival. Any `.c7` file written today must read the same way forever.

The format absorbs new data types. When a new device ships with a new `type` or `format`, it's added alongside the existing ones and nothing already written changes.

### Reserved `type` Names

The types `waveform` and `wavetable` are reserved for future wavetable devices. The Monomachine's DigiPro data is stored under `digipro` and `digipro_wavetable` specifically so that a device with a native wavetable engine can claim the reserved names later.
