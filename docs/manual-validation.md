# Remaining manual validation

Implementation and automated tests were completed on 2026-11-02. These checks are **not yet performed**. Use a separate test configuration if preserving an existing setup. Launch `target/release/rztop.exe` from an interactive Windows Terminal; the app requires a real terminal.

1. Compare the All preset with `../../layout-reference.png`: CPU full width, left GPU/Memory/Network/Disk, right CPU/Memory/I/O/GPU/VRAM, compact statusbar. Check portrait and narrow sizes, themes, clipping and terminal resize.
2. Hover and click visible controls: menu, presets, interval, widget numbers, GPU/network switching, network toggles, disk I/O, process sort/filter/pause, Options tabs/names/values/arrows/pages and close/save/cancel. Compare each to its keyboard action. Check hover leaves no underline or resize trails.
3. Drag both strokes of horizontal and vertical boundaries, nested stacks, and ranking boundaries. Drag to minima and back to start; cross a button during a drag. Verify captions/corners survive, paired arrows follow the pointer, hidden widgets leave no gaps, and statusbar stays one row. Right-click resets the relevant stack/all rankings. Resize terminal or press a key mid-drag to cancel.
4. Open modals/filter/Options text editing and ensure clicks cannot activate controls behind them. Confirm mouse release never repeats an action. Paste harmless text in normal and filter modes and verify it is ignored.
5. Confirm edits save only on completion, drag saves on release, canceled filters restore their original value, and values survive restart. Check a custom `--config` file remains the destination. To test import, use an isolated XDG_CONFIG_HOME and a disposable legacy config; verify source bytes unchanged and an existing destination is refused.
6. Observe I/O after at least two process samples; compare known busy processes and idle processes, test filter and pause. Rates represent general process I/O, not only disk throughput.
7. Verify rztop CPU appears beside uptime, continues updating when rankings pause or filter hides rztop, and disappears cleanly when width is insufficient. Click RAM and Commit in Memory ranking; verify descending ordering, marker and restart persistence.
8. On supported AMD hardware, compare observed boost and board/ASIC power reference units against driver reporting. Validate missing-reference behavior and manual scale overrides. Read `amd-references.md` before adjusting bindings.
9. Quit and confirm terminal cursor, raw mode, mouse capture and bracketed paste are restored.

Record terminal version, dimensions, hardware/driver, observations and any defects in `../../IMPLEMENTATION_STATUS.md`. Do not mark these checks passed merely because automated rendering tests pass.
