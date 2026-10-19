# Themes

The editor comes with three themes, and you can add your own.

## Built-in themes

**View ▸ Theme** lists them; the choice applies at once and is remembered
the next time you start the editor.

- **Dark**, the default.
- **Light**: the same colors, deepened so they read on light cards.
- **High Contrast**: white text on black at 7:1 or more everywhere it
  sits, white outlines on cards, buttons, and fields, thicker outlines,
  and a yellow ring around whatever has keyboard focus. Socket colors are
  brightened so each type still stands out.
- **Follow System** switches between Dark and Light with your system's
  setting while the editor runs.

## Your own themes

A theme is a small text file in the **themes folder**, next to the
editor's settings:

| System | Folder |
| --- | --- |
| Linux | `~/.config/audioconstructor/themes/` |
| macOS | `~/Library/Application Support/Audio-Constructor/themes/` |
| Windows | `%APPDATA%\Audio Constructor\config\themes\` |

**View ▸ Theme ▸ Open Themes Folder** opens it (making it if it isn't
there yet). Put a `.json` file in it, then choose **View ▸ Theme ▸ Reload
Themes**: your themes are listed under the built-in ones, by name. The
editor remembers the one you pick by its name, so editing the file and
reloading shows your changes at once. If the file goes away, the editor
shows Dark until it's back.

A theme file names itself, says which built-in theme it starts from, and
changes any of that theme's colors. Everything it doesn't mention stays
as the base has it, so a theme can be as short as this:

```json
{
  "name": "Midnight",
  "base": "dark",
  "colors": {
    "canvas_bg": "#05070A",
    "accent": "#FFB000",
    "type_audio": "#FF8800"
  }
}
```

| Field | What it holds |
| --- | --- |
| `name` | The name View ▸ Theme shows. It can't be the name of a built-in theme, and two files can't share one. |
| `base` | `"dark"`, `"light"`, or `"high_contrast"`: the theme to start from. A light base shows the theme in the editor's light mode, the others in its dark mode. `"high_contrast"` also brings the thicker outlines. Without one, the base is Dark. |
| `colors` | Colors to change, by the names in the tables below. |
| `header_mix` | How much of the category color goes into node headers, from `0` (none: headers are `header_base`) to `1` (all). Dark uses `0.42`, Light `0.2`, High Contrast `0.36`. |

Colors are written `"#RRGGBB"`, or `"#RRGGBBAA"` with an opacity from
`00` (clear) to `FF` (solid), as in web pages.

### When something is wrong

Nothing in a theme file can stop the editor. A file that isn't JSON, has
no name, or takes a built-in theme's name isn't listed; View ▸ Theme says
which file and why, and so does the status line at startup. In a file
that works, a misspelled color name (the menu suggests the right one) or
a color that doesn't read is skipped, the rest of the theme still
applies, and hovering the theme in the menu lists what was skipped.

The editor also checks that body text stays readable: if your text
colors have less than 4.5:1 contrast against a panel, a field, a button,
the canvas, or a node card (the level the built-in themes are held to),
the menu marks the theme **low text contrast** and choosing it says
which color is the problem.

## Every color

The tables list every color a theme can change, with what each one is
in the three built-in themes. A good way to start a theme is to copy the
column of the base you like into your file and change what you want.

### Canvas and panels

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `canvas_bg` | The canvas behind the nodes. | `#181A1F` | `#E9EBEF` | `#000000` |
| `grid_dot` | The dots of the canvas grid. | `#30343C` | `#C9CDD5` | `#404040` |
| `grid_dot_major` | Every fifth grid dot, a little stronger. | `#3C414A` | `#B4BAC4` | `#666666` |
| `panel_bg` | The side panel, menu bar, transport bar, and status bar. | `#202228` | `#F6F7F9` | `#000000` |
| `well_bg` | Recessed areas inside panels: text fields, meter wells. | `#15161A` | `#FFFFFF` | `#0A0A0A` |
| `border` | Separators and the outlines of panels and windows. | `#30333A` | `#D3D7DE` | `#C8C8C8` |
| `window_bg` | Windows, menus, and popups. | `#25282F` | `#FFFFFF` | `#000000` |
| `faint_bg` | Faint backgrounds, such as striped rows. | `#24272D` | `#EEF0F3` | `#141414` |

### Text

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `text` | Body text. | `#D4D8DF` | `#262A31` | `#FFFFFF` |
| `text_strong` | Titles and emphasized text, node titles too. | `#F2F4F7` | `#111418` | `#FFFFFF` |
| `text_muted` | Secondary text: shortcuts in menus, wired values, units. | `#959CA7` | `#586170` | `#E0E0E0` |
| `text_faint` | Hints and the faintest labels. | `#5E6570` | `#7A8290` | `#BDBDBD` |
| `accent` | Links, the active tab, and other highlights. | `#7CA7F5` | `#2560C4` | `#66B2FF` |
| `accent_dim` | The background of selected text and selected rows. | `#2E3D59` | `#DDE8FA` | `#0A2A4D` |
| `error` | Errors and the error badge. | `#F06A6A` | `#C02626` | `#FF8080` |
| `warning` | Warnings and the warning badge. | `#E8B04A` | `#945700` | `#FFC740` |
| `ok` | Confirmations: a baked node, a device that works. | `#5FC88F` | `#16733D` | `#5CE08A` |
| `code_bg` | The background of code in help text. | `#2C3038` | `#E8EBF0` | `#1A1A1A` |
| `code_text` | Code in help text. | `#E6C08A` | `#8A4B00` | `#FFD27F` |

### Node cards

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `node_bg` | A node's card. Slightly see-through, so wires behind it show dimmed. | `#282C32EA` | `#FDFDFEF0` | `#0A0A0A` |
| `node_outline` | A node card's outline. | `#383C45` | `#C3C8D0` | `#E0E0E0` |
| `node_selected` | The outline of a selected node. | `#E6EAF0` | `#2B303A` | `#00E5FF` |
| `node_hover` | The outline of the node under the pointer. | `#5A606B` | `#8A919C` | `#FFFFFF` |
| `header_base` | What node headers mix their category color into (see `header_mix`). | `#2B2E35` | `#FFFFFF` | `#000000` |
| `missing_header` | The header of a node whose kind this version doesn't know. | `#6A3A3A` | `#B05A5A` | `#8C1C1C` |
| `group_header` | The header of groups and of the group's input and output cards. | `#4C8C7A` | `#3D8270` | `#2E8C76` |
| `node_shadow` | The shadow under node cards. | `#0000006E` | `#00000028` | `#00000000` |
| `focus_ring` | The ring around whatever has keyboard focus: a node on the canvas, a button, a field. | `#7CA7F5` | `#2560C4` | `#FFD400` |

### Inline values and controls

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `widget_bg` | The wells of inline values on node cards. | `#1C1E23` | `#E6E9EE` | `#141414` |
| `widget_bg_hover` | An inline value's well under the pointer. | `#22252B` | `#DCE0E6` | `#262626` |
| `widget_fill` | The filled part of a drag slider. | `#344A66` | `#C5D6EF` | `#1F4A80` |
| `widget_fill_hover` | A slider's fill under the pointer. | `#3D5778` | `#B3C9E9` | `#285A99` |
| `raised` | Raised parts of node controls: button tops, knob caps. | `#32363F` | `#FFFFFF` | `#2A2A2A` |
| `raised_hover` | Those under the pointer. | `#3A3F49` | `#EEF1F5` | `#3A3A3A` |
| `raised_lip` | The lip under a raised button. | `#1A1C21` | `#C3C8D0` | `#BDBDBD` |
| `body_shadow` | The shade under knob caps, dots, and keys. | `#15161A` | `#A9B0BB` | `#000000` |
| `toggle_off` | A switched-off toggle's track. | `#30343C` | `#CDD2DA` | `#3A3A3A` |
| `toggle_hover` | A toggle's track under the pointer. | `#3A3F49` | `#BEC4CE` | `#4A4A4A` |
| `on_accent` | Marks drawn on the accent color, such as a checkbox's tick. | `#10141C` | `#FFFFFF` | `#000000` |

### Buttons and lists

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `button_bg` | Buttons at rest. | `#2E323A` | `#E3E6EB` | `#141414` |
| `button_weak_bg` | Quieter buttons (menu items, small buttons) at rest. | `#2B2E35` | `#E8EBEF` | `#0A0A0A` |
| `button_hover_bg` | A button under the pointer. | `#383D47` | `#D8DCE3` | `#2A2A2A` |
| `button_hover_weak_bg` | A quieter button under the pointer. | `#353942` | `#DCE0E6` | `#262626` |
| `button_hover_stroke` | The outline of a button under the pointer. | `#4A505B` | `#B3BAC5` | `#FFFFFF` |
| `button_active_bg` | A button while pressed, or while it has keyboard focus. | `#3F4653` | `#CBD2DD` | `#3A3A3A` |
| `button_open_bg` | A menu button whose menu is open. | `#333740` | `#DCE0E6` | `#262626` |
| `button_stroke` | The outline of buttons at rest. Transparent in Dark and Light. | `#00000000` | `#00000000` | `#BDBDBD` |
| `row_hover` | A row under the pointer in a list (Library, palette). | `#2B2F37` | `#E6E9EE` | `#262626` |
| `popup_shadow` | The shadow under menus and popups. | `#0000008C` | `#00000038` | `#000000A0` |

### Transport

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `record` | The record button while recording. | `#E84A4A` | `#D32F2F` | `#FF6060` |
| `play` | The play button while playing. | `#5FC88F` | `#16733D` | `#5CE08A` |

### Gestures and overlays

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `select_fill` | The inside of the box drawn while box-selecting. | `#6087CB40` | `#2560C530` | `#00E4FF30` |
| `select_stroke` | The edge of that box. | `#7CA7F5` | `#2560C4` | `#00E5FF` |
| `cut_line` | The knife line drawn while cutting wires. | `#F06A6A` | `#C02626` | `#FF8080` |
| `reroute_line` | The line drawn while adding reroutes (Shift+right-drag). | `#A9B4C2` | `#586170` | `#FFFFFF` |
| `breadcrumb_bg` | The group path shown at the canvas's top left inside a group. | `#2E403BE8` | `#D5ECE6E8` | `#00332B` |
| `hint_bg` | The hint shown at the canvas's bottom during a gesture. | `#131418D8` | `#FFFFFFE8` | `#000000` |

### Frames and notes

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `frame_fill` | The inside of a frame. | `#60697B70` | `#79869A26` | `#FFFFFF12` |
| `frame_border` | A frame's outline. | `#3A3F48` | `#B9BFC9` | `#BDBDBD` |
| `frame_label` | A frame's title. | `#A8AFBA` | `#4A515C` | `#FFFFFF` |
| `note_bg` | A Comment note's paper. | `#3B3726` | `#FFF3C4` | `#1A1600` |
| `note_header` | A Comment note's top strip. | `#4A442A` | `#F5E39C` | `#2A2400` |
| `note_text` | A Comment note's text. | `#ECE4C4` | `#3D3415` | `#FFF2B3` |

### Meters and scopes

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `meter_low` | Meters below −12 dB. | `#5FC88F` | `#2E9E5B` | `#5CE08A` |
| `meter_mid` | Meters from −12 to −3 dB. | `#E8C24A` | `#D9A520` | `#FFC740` |
| `meter_high` | Meters above −3 dB. | `#F06A6A` | `#D64545` | `#FF8080` |
| `meter_bg` | The empty part of a meter. | `#141519` | `#DADEE4` | `#1A1A1A` |
| `scope_bg` | A Scope's or Analyzer's background. | `#121417` | `#F3F5F8` | `#000000` |
| `scope_grid` | Their grid lines. | `#24282F` | `#DCE0E6` | `#404040` |

### Sockets and wires

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `socket_edge` | The ring around socket dots. | `#121316` | `#FFFFFF` | `#000000` |
| `wire_casing` | The thin edge under each wire, so crossing wires stay apart. | `#00000046` | `#FFFFFF96` | `#000000C8` |
| `wire_flash` | What flowing and flashing wires brighten towards. | `#FFFFFF` | `#1B1F27` | `#FFFFFF` |
| `type_audio` | Audio sockets and wires. | `#F0A030` | `#C46F00` | `#FFA94D` |
| `type_float` | Float sockets and wires. | `#6FB1E6` | `#2B7BC0` | `#66C2FF` |
| `type_int` | Int sockets and wires. | `#5FC88F` | `#1F8A55` | `#5CE08A` |
| `type_bool` | Bool sockets and wires. | `#E06C9F` | `#C2407D` | `#FF7AB8` |
| `type_trigger` | Trigger sockets and wires. | `#F2D15C` | `#977C00` | `#FFE14D` |
| `type_midi` | MIDI sockets and wires. | `#A98BF0` | `#7A55D6` | `#C3A6FF` |
| `type_spectrum` | Spectrum sockets and wires. | `#4FD1D9` | `#138A94` | `#4DF0F0` |
| `type_buffer` | Buffer sockets and wires. | `#C8A878` | `#9A7444` | `#E0C08A` |
| `type_text` | Text sockets and wires. | `#E8DFC0` | `#8E7E4A` | `#FFF5D6` |
| `type_any` | Reroutes before they're connected. | `#9AA0A8` | `#7A818C` | `#C8C8C8` |

### Node header colors by category

| Field | What it colors | Dark | Light | High Contrast |
| --- | --- | --- | --- | --- |
| `category_io` | Input / Output node headers. | `#C85555` | `#C04848` | `#FF6B6B` |
| `category_generators` | Generators node headers. | `#DB7E3E` | `#D06A22` | `#FF9A4D` |
| `category_level` | Level & Dynamics node headers. | `#C9A43E` | `#A8821E` | `#FFD04D` |
| `category_filters` | Filters node headers. | `#5CA65E` | `#3F9645` | `#6BE06E` |
| `category_time` | Time node headers. | `#3AA398` | `#22978B` | `#4DE0C8` |
| `category_spectral` | Spectral node headers. | `#3CB2C8` | `#1F9DB5` | `#4DD8FF` |
| `category_math` | Math & Logic node headers. | `#4E84D6` | `#3A72C8` | `#6BA6FF` |
| `category_modulation` | Modulation node headers. | `#6E66CC` | `#5B52C0` | `#9A8FFF` |
| `category_triggers` | Triggers & Gates node headers. | `#D6BE40` | `#A68A10` | `#FFE14D` |
| `category_sequencing` | Sequencing & Notes node headers. | `#986CD4` | `#8656C8` | `#C38FFF` |
| `category_controls` | Controls node headers. | `#D4649E` | `#C64F8C` | `#FF7AB8` |
| `category_display` | Display node headers. | `#828892` | `#7A818C` | `#B0B0B0` |
| `category_utility` | Utility node headers. | `#627386` | `#5A6B80` | `#8FA8C8` |
