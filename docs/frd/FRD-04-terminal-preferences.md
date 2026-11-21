# FRD-04 — Alacritty and tmux configuration parity

**Goal:** the embedded terminal looks and behaves like the user's own Alacritty and tmux setup, without modifying either config.

**Sources:**
- 2026-11-20T22:25Z: *"Use our system current default terminal configuration that's in our home directory."*
- 2026-11-20T22:36Z: *"have the GUI font match the font we use in alacritty"*
- 2026-11-20T22:41Z: *"use the same tmux configs as at ~/"*
- 2026-11-20T22:47Z: *"the terminal text feel zoomed… Goal is to get experience as close to current Alacritty machine configurations as possible"*

| ID | Priority | Requirement | Verification | Status |
| --- | --- | --- | --- | --- |
| CFG-01 | Must | The Alacritty config is found in Alacritty's own search order: `$XDG_CONFIG_HOME/alacritty/alacritty.toml`, `$XDG_CONFIG_HOME/alacritty.toml`, `~/.config/alacritty/alacritty.toml`, `~/.alacritty.toml`. A relative `XDG_CONFIG_HOME` is ignored. | `finds_config_in_alacritty_search_order` | Implemented |
| CFG-02 | Must | `general.import` (and legacy top-level `import`) are followed, including `~/` paths. Later files override earlier ones and the main file wins. Missing imports are skipped. Cycles are reported as an error. | `loads_imports_and_applies_local_overrides`, `rejects_cyclic_imports` | Implemented |
| CFG-03 | Must | Imported settings: font family, font size (5–72), `font.offset`, primary, normal, bright and dim colors, and `keyboard.bindings` entries that send literal `chars`. Unsupported or invalid entries are ignored individually. | `loads_imports_and_applies_local_overrides`, `defaults_to_dracula_and_ignores_unsupported_settings` | Implemented |
| CFG-04 | Must | Without a config, the terminal uses the Dracula palette. | `defaults_to_dracula_and_ignores_unsupported_settings` | Implemented |
| CFG-05 | Must | On macOS the font size is used as Core Text points (not converted to pixels), and the cell grid uses Alacritty's Core Text metrics, so text isn't zoomed relative to standalone Alacritty. | `loads_imports_and_applies_local_overrides` (size); Manual: compare cell size with standalone Alacritty (both 15×33 px on Retina). | Implemented |
| CFG-06 | Must | The GUI (sidebar, headers) uses the same font family as the terminal, at its own interface sizes. | Manual | Implemented |
| CFG-07 | Must | tmux reads the user's normal config (for example `~/.tmux.conf`). The app passes no `-f` and no option overrides, only terminal capability flags. | `isolates_sessions_without_overriding_home_config_and_passes_a_literal_cwd` | Implemented |
| CFG-08 | Must | The app never writes to the user's Alacritty or tmux configuration. | Manual (code review: no write paths) | Implemented |
| CFG-09 | Should | A config that fails to parse is shown as a notice. The app still starts with default preferences. | Manual | Implemented |

## Known differences from standalone Alacritty

- Glyphs are painted by iced, so antialiasing can differ slightly.
- `window.padding`, `dynamic_padding`, decorations, cursor and selection styling, Option-as-Alt and Alacritty-specific binding actions aren't imported.
