# Pulse MVP Manual Acceptance Checklist

This is the remaining human-only acceptance work as of 2026-08-01. Check an item only when its expected behavior is confirmed on the current build. Leave failures unchecked and add a short note or screenshot below the item.

Already verified and intentionally not repeated here: live Core Audio device enumeration, real-library scan counts/timing, cached and folder artwork rendering, Albums Grid/List switching, page-header sizing, and single-click arming and double-click playback basics.

Intentional MVP behavior: Shuffle and Repeat remain inert ([#32](https://github.com/yicheng47/pulse-src/issues/32)); the queue button now opens the post-MVP queue popover. Playlist reorder uses Move Up / Move Down rather than drag. Library text fields still support only the existing append/backspace path, without CJK IME composition or caret movement ([#35](https://github.com/yicheng47/pulse-src/issues/35)).

## Setup and data safety

- [x] Run the current branch with `make run` and confirm the app opens without a panic.
- [ ] Have one 16-bit / 44.1 kHz track and one 24-bit / 96 kHz FLAC available for hardware validation.
- [x] Use a disposable copied music root for rename, removal, cancellation, and forced-failure tests; do not change permissions on the live NAS library.
- [ ] Do not delete or modify the released app's `~/Library/Application Support/pulse` library database or cover cache; the `pulse-dev` profile is disposable and may be removed to exercise first run.

## Playback and physical hardware

- [ ] Drop a supported FLAC, ALAC-in-M4A, AIFF, or WAV anywhere in the window; playback starts and the row shows the correct title, elapsed time, duration, format, and artwork state.
- [ ] Pause and resume a dropped track; resume continues from the paused position rather than restarting at zero.
- [ ] Drag the progress bar and release inside and outside its visual track; one seek completes and playback continues at the dropped position.
- [ ] Drop an unsupported file; Pulse shows a visible error instead of silently doing nothing.
- [ ] Select the Matrix Mini-i Pro 4 and play the 16/44.1 track; output is clean, the DAC reports 44.1 kHz, and there is no noise or channel corruption.
- [ ] Play the 24/96 FLAC through the Matrix; output is clean and the DAC reports 96 kHz.
- [ ] Pause, resume, and seek during Matrix playback; all operations remain clean and keep the expected position.
- [ ] Stop playback, switch away from the Matrix, and quit Pulse; hog mode is released each time and another application can use the device.
- [ ] Select AirPods before playback and while another device is playing; audio starts or restarts cleanly without a hog-mode or nominal-rate failure.
- [ ] With AirPods selected, the capability line reads `Up to N kHz` without the unsupported-device amber message; repeatedly opening the popover does not duplicate a notice.

## Shell and macOS commands

- [ ] Confirm the sidebar brand and the macOS Dock/application icon match the Pencil app icon.
- [ ] Click Albums, Tracks, Playlists, Storage, and Devices; each click changes the body and moves active styling to exactly one sidebar item.
- [ ] Confirm the native traffic lights sit comfortably within the top header without crowding the Pulse brand and all three remain clickable.
- [ ] Drag the window from blank space across the top header and sidebar; it moves natively and double-click follows the macOS titlebar setting. Inside the search field, double-click does not zoom the window and press-then-move does not drag it.
- [ ] Press `Cmd+M`; the window minimizes.
- [ ] Press `Cmd+H`; Pulse hides and can be restored normally.
- [ ] Press `Cmd+W`; the window closes and Pulse exits rather than remaining stranded without a window.
- [ ] Relaunch and press `Cmd+Q`; Pulse quits.
- [ ] Confirm the macOS menu bar exposes Pulse, Edit, and Window menus and their visible commands match the shortcuts above.

## Output-device lifecycle and errors

- [ ] Select a different output while idle; the playback row updates to the confirmed device and shows honest capability text.
- [ ] Select a different output during playback; the track restarts cleanly on the new device and the row does not claim the switch before it succeeds.
- [ ] Quit and relaunch after selecting a non-default device; the saved UID resolves to the same device and the row reflects it.
- [ ] Switch away from the Matrix during playback; the Matrix releases hog mode and the new device becomes audible.
- [ ] Unplug or power off the selected device during playback; Pulse shows a clear error rather than hanging or continuing in silence.
- [ ] Reopen the device popover after unplugging or reconnecting hardware; the list refreshes to the current devices.
- [ ] If practical, make a target device unavailable or hogged by another process and select it; Pulse keeps UI and controller state consistent and shows the failure as plain visible text.

## Scratch first-run and library browsing

- [ ] Launch against the preserved scratch configuration; Albums and Storage show their designed empty states without stale data from the normal library.
- [ ] From Storage, choose Add Storage; the modal opens first, Choose… opens the native folder picker, and selecting the disposable music root returns its path to the same modal.
- [ ] The Add Storage name field receives keyboard focus immediately; Escape or Cancel closes the modal without adding a root.
- [ ] Add the disposable root with Scan now enabled; scan progress and counts update visibly, conflicting Add/Edit/Remove/Rename actions stay unavailable during the scan, and completion returns the row to Online.
- [ ] Cancel a scan in progress; already committed tracks remain usable, the UI exits the scanning state, and a later Rescan can complete normally.
- [ ] Open Albums after the scan; the expected albums, covers, counts, formats, years, genres, and durations are plausible for the disposable root.
- [ ] Open an album and use Play Album; playback starts from the first displayed track and the table remains below the header controls.
- [ ] Cycle every Albums sort and exercise All Albums, Hi-Res, Recently Added, and available genre chips; the section heading follows the active filter and visible ordering/filtering changes consistently.
- [ ] Cycle every Tracks sort and exercise All Tracks, Hi-Res, Recently Added, genre, and artist filtering; visible ordering/filtering changes consistently.
- [ ] Rename the disposable storage root; the new display name persists after navigation and relaunch without changing the filesystem path or losing tracks.
- [ ] Open Remove Storage and press Escape; the modal closes and the root plus its tracks remain.
- [ ] Confirm Remove Storage for the disposable root; its catalog rows disappear while the source music files remain untouched.

## Offline and scan-failure states

- [ ] With a scanned disposable or NAS root configured, disconnect or rename the root and Rescan; Storage shows Offline and existing catalog rows remain browseable rather than being deleted.
- [ ] Reconnect the offline root and Rescan; it returns Online and the retained rows reconcile normally.
- [ ] Make one disposable subfolder unreadable and Rescan; Storage shows the designed failed/error state, reports the failure honestly, and does not conservatively delete tracks whose absence could not be confirmed.
- [ ] Restore access and press Retry; the scan completes and the root returns to Online.
- [ ] After the storage and scan tests, drop a real supported track anywhere in the window; drop-to-play, row updates, pause/resume, and seeking still work.

## Queue and transport

- [ ] Open 菲靡靡之音, play from the first row, and let playback cross a track boundary; the next album track starts automatically with updated title, `artist - album`, artwork, progress, and format.
- [ ] While an album queue is active, press Next; it follows the visible album-detail order and becomes disabled on the final track.
- [ ] Let the final queue entry play to its natural end; playback stops cleanly with no wraparound, repeat, or unexpected restart.
- [ ] Within the first 3 seconds of a non-first track, press Previous; the previous queue entry starts.
- [ ] After more than 3 seconds of a track, press Previous; the current track restarts instead of moving backward.
- [ ] On the first queue entry, press Previous; the first track restarts and the index does not go negative.
- [ ] Start from a middle album row; the badge equals the number of later entries, decrements after each advance, and disappears at zero.
- [ ] In Tracks, choose a non-default sort and at least one filter, then play a middle visible row; Next follows only the current visible order and excludes filtered-out tracks.
- [ ] In Tracks with more than 50 matching rows, scrolling near the bottom appends backend pages without gaps or duplicate rows; changing sort, filter, or artist returns to the top and starts a fresh paged query.
- [ ] The Tracks scrollbar appears whenever loaded rows overflow and updates as backend pages append; grabbing anywhere along the thumb does not jump it under the cursor, rail clicks at the extreme top/bottom land exactly at the top/bottom, and releasing a bottom drag does not leave page loading churning.
- [ ] Play a row near a backend page boundary; Next and automatic advance continue beyond the currently loaded rows because the queue spans the full active sort/filter result.
- [ ] Drop a standalone file while a library queue is active; the prior queue and badge clear.
- [ ] Click Shuffle and Repeat; they remain intentionally inert and do not mutate playback order until #32 is implemented.

## Playlist lifecycle and ordering

- [ ] Create a playlist with an ASCII name; it appears in the list, becomes selected, and shows an empty detail panel.
- [ ] Submit an empty playlist name and press Escape from the name dialog; both paths cancel without creating or renaming anything.
- [ ] Create two playlists with the same name; both remain distinct entries.
- [ ] Rename a playlist from both the playlist-row context menu and the detail action; the same dialog swaps to Rename title/button text and persists the new name after navigation and relaunch.
- [ ] Open Delete Playlist and use Cancel and the header X; both paths preserve the playlist and all entries.
- [ ] Delete a playlist containing tracks; the confirmation states the exact entry count, deletion removes only the playlist, and the music files/tracks remain in the library.
- [ ] Right-click a track in Album Detail and add it to an existing playlist; the entry appears at the end.
- [ ] Right-click a track in Tracks and add it to an existing playlist; the entry appears at the end.
- [ ] Right-click a track in playlist detail and add it to an existing playlist; duplicate entries are allowed, including duplicates in the same playlist.
- [ ] Choose New Playlist… from the Add to Playlist flyout; the name dialog creates the playlist and automatically adds the originating track.
- [ ] Hover and click Add to Playlist; the flyout opens in either path, Escape dismisses it, and an outside click dismisses the menu.
- [ ] In playlist detail, choose Remove from Playlist; only that positioned entry disappears and remaining entries stay in order.
- [ ] Use Move Up and Move Down on first, middle, and last entries; disabled boundary actions cannot move past the ends, order updates immediately, and the order persists after navigation or relaunch.
- [ ] Put the same track in a playlist twice and play through both entries; queue order preserves both duplicates.
- [ ] Single-click a middle playlist row, then press the central Play button or the playlist Play button; playback starts at the selected row and the queue contains the later playlist entries in order.
- [ ] With no selected playlist row, press the playlist Play button; playback starts at the first entry.
- [ ] Double-click a playlist row or choose Play from its context menu; playback starts immediately from that positioned entry.
- [ ] Open track and playlist context menus near the top bar, bottom edge, and right edge; anchors align with the clicked row under the unified titlebar and every required item remains usable without clipping.

## Search

- [ ] Click the search field and press `Cmd+F`; both paths focus it, and `Cmd+F` preserves an existing query.
- [ ] Type 王菲 with the macOS CJK IME, including editing and backspace during marked-text composition; the final query is correct with no doubled insertion or deletion.
- [ ] Search 王菲; matching Albums and Tracks appear, group headers are honest, and no false No Matches state flashes during the debounce.
- [ ] Search `frank`; matching is case-insensitive across track title, artist, album, album artist, and playlist name as applicable.
- [ ] Search gibberish; all empty groups show `— NO MATCHES` and the plain no-results line names the query.
- [ ] Use Up and Down across a full result set; selection wraps, remains visible when reaching the final playlist rows, and never lands on a group header.
- [ ] Press Enter on an album result; search dismisses and Album Detail opens for that album.
- [ ] Press Enter on a track result; playback starts and its queue is the track's album in album order, so Next advances within that album.
- [ ] Press Enter on a playlist result; search dismisses and Playlists opens with that playlist selected.
- [ ] Press Escape with the popover open; it dismisses and blurs the field. Clicking outside does the same.
- [ ] Clear the query; the popover closes. Re-focus a retained non-empty query after dismissal; its results can open again normally.

## Visual comparison and current screenshot fix

- [ ] Compare the overall shell and playback row against Pencil `E3N1P` and `qKkw7`; spacing, colors, fonts, controls, titlebar, sidebar, and docked-row geometry have no obvious regressions.
- [ ] Compare Album Detail against `XaIfA`; cover/header/actions/table rhythm match, the `#` header is aligned with row numbers after the latest fix, and playing/selected row treatments are correct.
- [ ] Compare Tracks against `KfJr9` and artist-filter state `VSy1Y`; table columns, chips, selection, playing treatment, continuous scrolling, and the slim scrollbar are aligned, with no pagination footer.
- [ ] Compare Albums against `E3N1P`; responsive cards keep their minimum width and spacing, and continuous scrolling loads more cards without pagination chrome.
- [ ] Compare populated and empty Playlists against `MHrLm` and `LNdSq`; list/detail balance, covers, badges, controls, empty state, and queue rows are aligned.
- [ ] Compare playlist name dialog, track context menu/flyout, and search popover against `Lr7A3`, `cYA3i`, and `A3JuUc`; titles, buttons, grouping, selected rows, dismissal behavior, and overlay placement are aligned.
- [ ] Compare Storage empty, populated, scanning, offline, and failure states against `C6IrDC` and its state variants; no text overlaps, clipping, or stale controls remain.
- [ ] Compare the output-device popover against `vH78z`; device selection, active styling, capability copy, capture-phase toggle behavior, and notices look correct.

## Completion

- [ ] Restore the normal Pulse application-data directory and confirm the real library, cover cache, playlists, and saved output device are intact.
- [ ] Record every unchecked failure with reproduction steps and a screenshot where visual; do not mark the MVP manual pass complete until failures are fixed or explicitly accepted.
