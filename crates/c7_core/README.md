This crate contains all the hardware-protocol and audio-utility backend logic that C7 uses.

Something neat about having a separate `c7_core/` and `c7_app/` structure is that future projects can re-use any of the functions without needing to load the UI.