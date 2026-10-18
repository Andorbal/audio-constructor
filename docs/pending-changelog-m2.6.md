# Pending changelog entries: M2.6 examples

Entries for the M2.6 release section from the examples work. The M2.6
branch moves them into its section of `CHANGELOG.md` (and the decision at
the end into `docs/DECISIONS.md`, numbered there) and deletes this file.

### Added

- Seven new example projects, each a working patch with notes on the canvas
  saying what each part does and what to try. All of them play when you
  press Space, and render with `audioc render`, without any hardware:
  - **Sidechain Pump**: a pad that ducks under a four-on-the-floor kick
    through the Sidechain Ducker block, with the release set in beats so
    the pump follows the tempo, and a switch to hear it without.
  - **Drum Machine**: the Drum Machine block playing from the transport,
    and its per-drum triggers playing a second kit of Samplers cut from
    one audio file; a switch flips between the two kits.
  - **Multiband Compressor**: a four-track loop through the Multiband
    Compressor block, with before and after meters, a bypass switch, and a
    meter per band showing how hard each band is turned down.
  - **Vocoder**: a saw chord made to talk by the Vocoder block, with a
    robot voice built from a buzz and two moving formant filters; switch
    on Audio In to vocode your own voice.
  - **Generative Patch**: two Euclidean rhythms, random notes snapped to a
    scale, a plucked string and an FM bass. It plays the same tune every
    time until you change the project's seed.
  - **Tape Loop**: phrases recorded onto a four-second loop that saturates,
    darkens, and wobbles with wow and flutter on every pass until they
    fade.
  - **Guitar-to-Synth**: a synth that follows the pitch and dynamics of a
    guitar on Audio In, with a recorded riff standing in until you switch
    the guitar on.

### Changed

- The **Reverse Delay** example is the real thing now, not a placeholder:
  a melody whose echoes play backwards, built once on a Clip, where
  Reverse bakes and plays, and once on Audio In, where it switches itself
  off while live, with notes on the canvas explaining why.

## For docs/DECISIONS.md

| # | Decision | Alternatives considered | Why |
| --- | --- | --- | --- |
| D? | **Examples that need a live input carry a stand-in, picked with a Toggle and an audio Switch that default to it**: a recorded riff for Guitar-to-Synth, a synthesized robot voice for the Vocoder, a melody Clip for Reverse Delay. Examples copy shipped blocks exactly (a test compares them with `blocks/`), and a test lays every example out as the editor draws it and fails on overlapping cards or frames that cut through one. | Live-only examples (like Guitar Pedalboard); a separate demo project per example. | An example that is silent until hardware is set up teaches nothing on first open, and `audioc render` and the tests can only hear what doesn't need a device. One project keeps the live path and the stand-in side by side, which is itself a lesson. Exact block copies mean opening an example never offers an update or teaches an outdated block. |
