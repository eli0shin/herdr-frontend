# Parity audit against the feature inventory

Ticket 033 checked every **[Eli]** and **[Shipped, used]** item of
`docs/research/feature-inventory.md` §0–§19, its keybinding table (§22) and the regression checks
in its appendix against herdr-frontend. This page records the result for each one, with its
evidence.

## How it was checked

- **Side by side, the whole app.** `just e2e` ran herdr-frontend and Herdr app 0.1.18 through the
  same 20 scenarios (`crates/app/tests/e2e/scenarios/`). Each app had its own isolated
  herdr-core with the same seeded content, and its own isolated home (Herdr app got a copy of
  Eli's config, `docs/e2e.md`). In the run on this branch's head (`20261127-095629`), every step
  of both apps ran and was captured. The screenshots, one image per step with herdr-frontend on
  the left and Herdr app on the right, are in [`images/parity/`](images/parity/), named
  `<scenario>--<step>.jpg`. Each is the run's two PNGs scaled to 720 px high and joined
  (`magick <frontend.png> -resize x720 ( -size 10x720 xc:#5a5a5a ) ( <herdr-app.png> -resize x720 )
  +append -quality 80`).
- **Headless and isolated-core tests** for what a screenshot can't show: key handling, shared
  labels, the layout document, timing. The evidence column names them as `file::test` (`app/` is
  `crates/app/tests/`, `model/` is `crates/model/tests/`, `src/` names unit tests).
- **Terminal rendering against Ghostty**, not Herdr app (`AGENTS.md`). `just ghostty-diff` draws
  the same bytes in Ghostty.app (Eli's settings for `styles`) and in herdr-frontend's renderer and
  compares them cell by cell ([Terminal rendering](#terminal-rendering-against-ghostty)).

Verdicts:

- **Pass:** herdr-frontend does what the item says.
- **Intentional:** it differs on purpose: one of the scope decisions (`AGENTS.md`,
  `docs/plan.md`), a Herdr app workaround that isn't ported, or a Herdr app limit that is fixed.
- **Fixed:** a gap this ticket closed.
- **Follow-up:** a gap too large for this ticket, filed as its own ticket.
- **Out of scope:** shipped but not something herdr-frontend targets (Linux and Windows).

Differences that come from the harness, not from either app:

- The Herdr app window is tiled by AeroSpace (756 × 949 pt), while herdr-frontend's opens at
  1200 × 800 pt. Wrapping and the number of rows that fit differ for that reason alone.
- Herdr app names a terminal that never set a title by its address, so the adopted fake worker
  reads "t11" in its Agents list. herdr-frontend reads "Pi": an untitled terminal has no title,
  and the program name stands in for it, as the inventory's row-title rule says. Real agents
  always set a title.

## Summary

Every item passes, differs on purpose, or was fixed here, except four gaps filed as their own
tickets, all now done:

- **040** dead-space sweep (§3, and its toast in §8): done (`docs/dead-spaces.md`);
- **041** space names and branches from git checkouts (§3, and the space picker's branch matching): done;
- **042** kitty graphics (§17): done, with its colors in sRGB on purpose;
- **043** the peek's find bar, terminal menu and Cmd-hover address underline (§15, §10): done.

Fixed in this ticket: the lone tab pill (§1), the Agents list's icon colors (§2), the hollow
cursor under popups (§1), `cmd-shift-l` (§1), the picker's spinner and the keys help's header
(§10), and the "Terminal failed to start" toast and the login card (§8).

## §0 Interop contract

| Item | Verdict | Evidence |
| --- | --- | --- |
| Layout document: newest write wins, per-screen view state stays local, mod props kept | Pass | `herdr-shared/tests/golden.rs::decode_and_encode_give_back_the_cores_exact_bytes`, `::an_edit_writes_only_what_it_changed`; `model/layout.rs::an_edit_racing_another_clients_write_is_replayed_on_its_copy`, `::unknown_node_kinds_and_mod_blocks_survive_this_screens_writes`, `::another_clients_edit_shows_and_view_choices_survive_where_their_ids_do`; e2e `00-interop` (both apps list the same spaces after every edit) |
| Terminal adoption labels: an unknown space or tab is created unnamed, without focus | Pass | `model/layout.rs::a_labeled_terminal_no_pane_shows_is_adopted_without_taking_focus`; [`00-interop--01-adopt-a-labeled-worker`](images/parity/00-interop--01-adopt-a-labeled-worker.jpg): "worker-space" appears in both, the shown space stays |
| App labels `herdr.pinned`, `herdr.right-click`, `herdr.seen` | Pass | `model/agents.rs::pinned_agents_come_first_under_their_own_header`; `app/input.rs::right_clicks_go_to_the_program_only_when_its_terminal_says_so`; `model/popups.rs::the_terminal_menu_splits_zooms_routes_right_clicks_and_sends_sub_agents_back`; `model/attention.rs::looking_at_an_agent_shares_herdr_seen_once_per_turn`, `::done_turns_idle_when_another_client_looks` |
| Eli's labels: the Neovim bridge, `dotfiles.review`, `dotfiles.pi_pr_watch`, `dotfiles.pi_session`/`pi_shell` | Pass | Bridge: `model/layout.rs::neovim_*`, `app/isolated_core.rs::ctrl_l_past_a_dead_neovim_clears_its_label_and_moves_the_pane`. Review: `model/peek.rs::cmd_r_*`. PR watch: `src/agents.rs::pi_watch_labels_are_trusted_only_on_pi`, `model/agents.rs::pi_workers_nest_under_their_orchestrator_while_it_or_a_worker_is_focused`. herdr-frontend never reads or writes the Pi resume labels |
| App-local shared state | Intentional | The keys that apply here (`space.colors`, `agents.sort`, `dotfiles.agents.order`, `dotfiles.agents.hidden-machines`, the sidebar's width and shown state) live in herdr-frontend's own `shared.json`, never Herdr app's (`AGENTS.md`, `docs/sidebar.md`). `layout`, `workspace.panels`, `frame.welcomed` and `dotfiles.agents.jumping` have nothing to configure here: there is one layout, no panels, no welcome frame, and the jump bar isn't stamped |
| CLI surface tools drive (`ls | do | status`) | Pass | `app/window_control.rs::the_window_serves_every_cli_command_on_a_live_core`; e2e `00-interop` drives `space.new {focus:false}`, `space.rename`, `tab.rename` and `layout.switch` through both apps' CLIs. `navigator-status.set-shared` isn't served: live-reloaded settings replace it (§2). Each tool's own calls: `app/tool_calls.rs`, the selection file `docs/cli.md` "Frontend selection" (ticket 035) |

## §1 Window chrome and layout regions

| Item | Verdict | Evidence |
| --- | --- | --- |
| No system title bar; the tab bar and the sidebar's top row drag the window and double-click zooms | Intentional | `crates/app/src/window.rs` `CHROME` (borderless, no titlebar band); `.window_drag()` rows in `view/tabs.rs` and `view/sidebar.rs` drag it (controls keep their presses); every screenshot. A double click there does nothing (`window.rs` `no_zoom`): macOS's zoom fills the display's visible frame, under AeroSpace's bar and over its layout, and Ghostty's borderless window has no title bar to double-click |
| No macOS traffic lights | Intentional | None at all (`traffic_lights: None`). Herdr app parks them at x = −80 because it can't hide them; that workaround isn't ported (`AGENTS.md`). The tab bar keeps its 10 pt inset |
| Bare panes, a 1 px border-colored gap between splits only, focus shown by the cursor | Pass | [`01-window-chrome--01-split-tab`](images/parity/01-window-chrome--01-split-tab.jpg) |
| No status bar row | Pass | Terminals reach the window's bottom in every screenshot |
| Sidebar lists agents only; `cmd-b` / `ctrl-g b`; no dividers; drag 200–600 | Pass | `app/sidebar.rs::cmd_b_and_ctrl_g_b_toggle_the_sidebar`, `::the_edge_drags_the_width_between_200_and_600`, `::a_fresh_home_starts_from_the_defaults_and_keeps_its_own_after` (366 pt, shown); [`01-window-chrome--02-hide-the-sidebar`](images/parity/01-window-chrome--02-hide-the-sidebar.jpg) |
| 38 px rows, 6 above the 29 px tabs and 3 below | Pass | `app/tabs.rs::the_strip_shows_the_shown_tabs_as_eli_tabs_draws_them` |
| A lone tab's pill | Fixed | A space with one shown tab drew its pill across the whole strip (Rune's scroll viewport made the lone pill its full-width content). The pills now sit in a row of their own: `app/tabs.rs::a_lone_tab_is_a_pill_as_wide_as_its_contents`; compare [`01-window-chrome--00-start`](images/parity/01-window-chrome--00-start.jpg) |
| Window state persists; `ctrl-g d` closes the window, terminals keep running | Pass | `window.json` keeps the size and position the window closed at, and a place mostly off every display (AeroSpace's parking corner, a display that's gone) reopens centered (`src/window.rs::a_placement_reads_back_and_opens_the_window_there`, `::a_place_mostly_off_every_display_is_not_reopened`). It never reopens zoomed or full screen; under a tiling window manager the tiler sizes it; `window.close` is `tx.close_window()`, and core keeps the terminals |
| Terminal grid centering | Fixed | Herdr centers the grid, leaving part of a row above and below. The leftover now goes above the grid, so the bottom row sits on the pane's bottom edge, and split panes are whole cells wide and tall, so only the panes at the window's right and top edges have any. In a nested split, a pane beside one split again the same way keeps that split's 1 pt gaps (`app/shell.rs::split_panes_are_whole_cells_and_each_grid_sits_at_its_bottom_left`, `model/geometry.rs::nested_splits_keep_their_panes_whole_cells`) |
| Escape hatch `ctrl-alt-shift-escape` | Pass | `app/keymap.rs::the_escape_hatch_brings_elis_keys_back_and_sets_the_file_aside`, `model/modes.rs::the_escape_hatch_works_in_every_mode`. There are no layouts to set aside |
| Layout switch `cmd-shift-l` | Fixed | It showed a "Not yet" toast. `eli` is the only layout, so it now stays on `eli` quietly, as `do layout.switch` does: `app/keymap.rs::unbuilt_actions_show_a_not_yet_toast_that_goes` |
| Cursor while a popup has the keyboard | Fixed | The focused pane kept a solid cursor under popups and dialogs; Herdr app draws it hollow, as Ghostty draws an unfocused surface. Now hollow until the popup closes: `app/popups.rs::the_palette_filters_runs_a_command_and_remembers_it`; compare [`07-agent-navigation--05-jump-bar`](images/parity/07-agent-navigation--05-jump-bar.jpg) |

## §2 Look: theme, fonts, settings

| Item | Verdict | Evidence |
| --- | --- | --- |
| Arctic theme | Pass | `app/look.rs::with_no_settings_file_the_window_is_elis_arctic_at_zoom_1_1`; the canvas reads (30, 30, 30) = `#1e1e1e` in both apps' screenshots |
| Settings (zoom 1.1, 17 pt, ligatures off, metrics 1.0, minimum contrast 1, no scrollbars, dark; `copy_on_select`, OSC 52, `option_as_alt` at their defaults) | Pass | `docs/settings.md` defaults; `app/scrollback.rs::select_all_takes_the_scrollback_and_copy_on_select_can_be_off`; `terminal/tests/vt.rs::clipboard_writes_are_capped`; `app/input.rs::keys_go_with_their_physical_key_and_mods`. Sounds and telemetry don't exist |
| Fonts with corrected weights | Intentional | The installed Operator Mono's Book, Bold and italics, each style its own face, found as Ghostty finds them (ticket 039, `docs/settings.md` "Fonts"). Herdr app's prepared "Operator Mono Book Herdr" family is its workaround and isn't used. Herdr app's terminal text looks a little heavier in the screenshots; the reference is Ghostty ([below](#terminal-rendering-against-ghostty)). The panels draw in Operator Mono Medium, which gives them the weight of Herdr app's font-smoothed Book (`docs/settings.md` "Why the panels are Medium") |
| Program look: Pi purple, Neovim's full-color logo, tab bar glyphs in the program's color | Pass | `app/look.rs::tabs_show_their_programs_icon`; e2e tab pills |
| Agents list icons | Intentional | The Agents list, the PR finder and the peek draw a one-color glyph in the program's color, as the tab bar does (Pi purple, `view::shell::program_icon`). `eli-agents`, `pr-finder` and `peek` draw them in the foreground (`icon(p, size, t.foreground)`); Eli prefers the purple Pi. Compare [`06-agents-list--00-start`](images/parity/06-agents-list--00-start.jpg) |
| Chrome sizes at the same zoom | Fixed | Zoom and the panels' 13 pt text were already Herdr app's, but tab pills drew 2 pt narrower and Agents rows 2 pt shorter (Eli's v0.1.1 feedback): GPUI's 1 pt border takes room in an auto-sized box, and a Rune border takes none unless it's set to (`border_layout`). The pills' and rows' borders now take room (`view::tabs`, `view::sidebar`), so both measure as Herdr app's: a 51 pt row at zoom 1 (`app/sidebar.rs::the_sidebar_lists_agents_under_its_top_row`, `app/tabs.rs::a_lone_tab_is_a_pill_as_wide_as_its_contents`) |
| Zoom keys | Pass | `app/look.rs::the_zoom_keys_zoom_everything_and_write_settings_json`; [`02-look--01-zoom-in`](images/parity/02-look--01-zoom-in.jpg) |
| Settings window | Intentional | herdr-frontend is set up by files: `cmd-,` lists them and opens one in the editor (`docs/popups.md`), and the Machines dialog covers Settings → Machines (§9). [`02-look--03-settings`](images/parity/02-look--03-settings.jpg) |
| Preferences applied without a restart | Pass | `app/look.rs::settings_and_theme_files_reload_live`, `app/keymap.rs::the_key_file_in_the_home_applies_live_and_a_bad_one_keeps_the_keys`: `dot stow` can write the files and the window follows (`docs/settings.md` "Live reload") |

## §3 Spaces

| Item | Verdict | Evidence |
| --- | --- | --- |
| Space model: one machine each, order, rename to "" goes back, last tab closes the space | Pass | `model/control.rs::renames_write_trimmed_names_and_empty_goes_back_to_automatic`; `model/layout.rs::closing_a_spaces_last_tab_asks_and_confirming_closes_the_space`, `::remote_spaces_qualify_ids_and_launch_on_their_machine` |
| An unnamed space is called after its first terminal's repo | Pass (041) | Its first terminal's git checkout top, looked up over `machine.exec` on its machine (`docs/popups.md`): `model/checkouts.rs::unnamed_spaces_are_called_after_their_first_terminals_checkout` and the rest of that file, `app/popups.rs::an_unnamed_space_shows_its_checkout_and_the_space_picker_finds_its_branch`; e2e `03-spaces` (a shell in `api/crates/app` reads "api" in both apps) |
| Space picker (`ctrl-g s`) | Pass | `app/popups.rs::the_space_picker_ranks_and_goes_to_a_terminal_tab`; `model/popups.rs::the_space_picker_numbers_spaces_and_ranks_names_above_folders`, `::going_to_a_space_shows_a_terminal_tab_when_its_selected_one_is_an_agents`; [`03-spaces--04-space-picker`](images/parity/03-spaces--04-space-picker.jpg). A space's git branch matches too (041): `model/checkouts.rs::the_space_picker_matches_the_branch_of_the_terminal_a_space_shows_first`, e2e `03-spaces` |
| Select by number `ctrl-1…9` | Pass | `model/layout.rs::spaces_step_and_select_by_number_in_order`; [`03-spaces--02-ctrl-3-selects-space-3`](images/parity/03-spaces--02-ctrl-3-selects-space-3.jpg) |
| Next/previous space | Pass | same test; [`03-spaces--03-next-space`](images/parity/03-spaces--03-next-space.jpg) |
| Rename space `shift-f2` | Pass | `model/layout.rs::renaming_a_space_writes_its_name_and_empty_goes_back_to_automatic` |
| Close space `ctrl-g x` asks first | Pass | `app/popups.rs::closing_a_space_asks_and_cancel_or_confirm_answers`; [`03-spaces--08-ask-before-closing-a-space`](images/parity/03-spaces--08-ask-before-closing-a-space.jpg) |
| Space colors | Pass | `src/agents.rs` color tests; `model/agents.rs` rows; the picker's avatars ([`10-pickers--01-tab-picker`](images/parity/10-pickers--01-tab-picker.jpg)) |
| Close spaces whose folder was deleted | Pass | Built in 040 (`docs/dead-spaces.md`), with the mod's current rule: only a working agent keeps a space. `model/dead_spaces.rs` ports the mod's check case by case; the e2e scenario `03-spaces` deletes an adopted worker's folder and both apps close its space |
| `repos` integration | Pass | `model/control.rs::a_new_space_without_focus_waits_behind_the_shown_one_and_answers_its_ids`; `ls` reports each space's folder |

## §4 Tabs

| Item | Verdict | Evidence |
| --- | --- | --- |
| Tab bar: 29 px pills, radius 7, 12.8 px, ≤ 170 px, icon, spinner, dot, " Z", × on the active one, scrolls | Pass | `app/tabs.rs::the_strip_shows_the_shown_tabs_as_eli_tabs_draws_them`, `::a_zoomed_tab_reads_z`; [`05-panes--05-zoom`](images/parity/05-panes--05-zoom.jpg) |
| Agents' tabs stay out of the bar (the review's too) | Pass | `model/tabs.rs::unnamed_tabs_are_numbered_among_the_shown_tabs_and_pi_tabs_take_their_session`; `model/layout.rs::tab_keys_count_and_step_over_only_shown_unsettled_tabs`; [`06-agents-list--03-pi-needs-you`](images/parity/06-agents-list--03-pi-needs-you.jpg) |
| Unnamed tabs numbered among the shown tabs | Pass | same |
| Tab names from Pi | Pass | same, and `model/tabs.rs::a_suffix_someone_already_used_gets_a_further_one` |
| Mouse on tabs | Pass | `app/tabs.rs::a_click_shows_the_tab_and_its_panes_take_the_keyboard`, `::a_press_released_on_another_pill_moves_the_tab_there`, `::a_double_click_renames_and_a_right_click_opens_the_tab_menu`, `::the_wheel_steps_through_the_shown_tabs` |
| Machine label at the bar's end | Pass | every screenshot ("This computer"); `app/machines.rs::another_machines_spaces_and_agents_show_beside_this_ones` |
| No Files/Changes buttons | Pass | none exist |
| Window buttons where the OS draws none | Out of scope | macOS only (`AGENTS.md`) |
| Numbered tabs skip hidden tabs | Pass | `model/layout.rs::tab_keys_count_and_step_over_only_shown_unsettled_tabs`; [`04-tabs--05-tab-1-by-number`](images/parity/04-tabs--05-tab-1-by-number.jpg) |
| Next/previous tab skips hidden and settled tabs; on an agent's tab it walks the Agents list | Pass | same, and `::tab_keys_on_an_agents_tab_move_through_the_agents`; [`04-tabs--02-next-tab`](images/parity/04-tabs--02-next-tab.jpg) |
| Focus after closing a tab | Pass | `model/layout.rs::closing_a_tabs_last_pane_closes_the_tab_and_eli_goes_left`, `::closing_the_first_tab_shows_the_new_first_and_settled_and_agent_tabs_are_skipped`, `::closing_an_agents_tab_focuses_the_next_agent_across_spaces`; `app/tabs.rs::the_close_button_closes_its_tab_and_eli_shows_the_one_to_its_left` |
| Jump list `ctrl-g o` / `ctrl-g i` | Pass | `src/view.rs` jump-list tests; `model/modes.rs::the_jump_list_goes_back_and_forward_over_tabs_seen_for_a_second` |
| Tab commands (`cmd-t`, `cmd-w`, `f2`; closing the last tab asks) | Pass | `model/layout.rs::a_new_tab_starts_a_labeled_terminal_then_goes_after_the_selected_tab`, `::renaming_a_tab_prompts_with_its_name_and_writes_it_trimmed`; [`04-tabs--04-new-tab`](images/parity/04-tabs--04-new-tab.jpg), [`04-tabs--07-rename-prompt`](images/parity/04-tabs--07-rename-prompt.jpg) |
| Lone pill | Fixed | see §1 |

## §5 Panes, splits and zoom

| Item | Verdict | Evidence |
| --- | --- | --- |
| Split right/down, on the pane's machine | Pass | `model/layout.rs::splitting_puts_the_new_pane_beside_the_focused_one_and_focuses_it`, `app/isolated_core.rs::a_split_made_here_shows_in_the_document_as_herdr_app_writes_it`; [`05-panes--04-split-right`](images/parity/05-panes--04-split-right.jpg) |
| Close pane | Pass | `model/layout.rs::closing_a_pane_kills_its_terminal_and_focus_goes_back`; [`05-panes--08-close-w2`](images/parity/05-panes--08-close-w2.jpg) |
| Zoom (`cmd-shift-enter`) | Pass | `model/layout.rs::zoom_shows_one_pane_moves_with_focus_and_ends_with_one_pane_left`; [`05-panes--05-zoom`](images/parity/05-panes--05-zoom.jpg), [`05-panes--06-unzoom`](images/parity/05-panes--06-unzoom.jpg) |
| Focus by direction | Pass | `model/layout.rs::focus_by_direction_prefers_the_pane_last_focused_along_the_edge`; [`05-panes--03-focus-down`](images/parity/05-panes--03-focus-down.jpg) |
| Neovim-aware Ctrl+H/J/K/L | Pass | `model/layout.rs::neovim_panes_hand_ctrl_hjkl_to_neovim_over_its_server`, `::neovim_at_its_edge_asks_for_the_pane_move_once_and_only_while_focused`, `::a_labeled_neovim_that_is_gone_loses_its_label_and_the_pane_moves`, `::nvim_that_cant_run_says_so_and_nothing_moves`, `::a_remote_neovim_gets_its_keys_on_its_own_machine`; `app/isolated_core.rs::ctrl_l_past_a_dead_neovim_clears_its_label_and_moves_the_pane` |
| Swap | Pass | `model/layout.rs::swapping_moves_the_pane_not_the_focus` |
| Resize from the keyboard (40 px) | Pass | `model/layout.rs::a_keyboard_resize_moves_the_nearest_divider_forty_points`; [`05-panes--07-resize`](images/parity/05-panes--07-resize.jpg) (both move the divider left by 40) |
| Drag to rearrange | Pass | `app/panes.rs::a_panes_grip_shows_on_its_top_edge_and_drops_it_onto_another_pane`, `::a_grip_dropped_on_an_edge_places_the_pane_beside`, `::a_short_press_on_the_grip_moves_nothing_and_a_zoomed_tab_has_no_grip` |
| Drag dividers | Pass | `app/panes.rs::dragging_a_divider_resizes_the_panes_beside_it_and_writes_the_sizes` |
| Pane rename | Pass | `app/popups.rs::the_terminal_menus_rename_asks_for_the_panes_name`, `app/isolated_core.rs::a_pane_renamed_here_shows_in_the_document_as_herdr_app_reads_it` |

## §6 Agents list

| Item | Verdict | Evidence |
| --- | --- | --- |
| Header row: "Agents", the machine button ("N hidden"), the sort button | Pass | `app/sidebar.rs::the_sidebar_lists_agents_under_its_top_row`; `app/machines.rs::the_header_button_opens_the_filter_and_a_click_toggles_a_row` |
| Sort menu | Pass | `app/sidebar.rs::the_sort_menu_picks_by_click_or_keys` |
| Sections: Pinned, Needs you / Done / Working / Idle with counts, or space headers | Pass | `model/agents.rs::priority_sorts_by_rank_into_sections_with_counts_and_numbers_every_row`, `::grouped_view_heads_each_space_in_model_order_and_leaves_the_space_out_of_rows`, `::pinned_agents_come_first_under_their_own_header`; [`06-agents-list--03-pi-needs-you`](images/parity/06-agents-list--03-pi-needs-you.jpg) |
| Clicking a header folds it | Intentional | Headers never fold: Eli folds them by accident (`docs/plan.md` decision). `app/sidebar.rs::the_icon_or_chevron_folds_sub_agents_and_headers_never_fold` |
| Rank (parents take sub-agents' and workers' rank) | Pass | `model/agents.rs::sub_agents_nest_with_tree_lines_lift_their_parent_and_fold`, `::pi_workers_nest_under_their_orchestrator_while_it_or_a_worker_is_focused`; [`08-attention--04-done-needs-you`](images/parity/08-attention--04-done-needs-you.jpg) (a done Pi under Needs you because its sub-agent is blocked, in both) |
| A viewed done agent stays in Done until you leave | Pass | `model/agents.rs::a_viewed_done_agent_stays_in_done_until_you_leave_it` |
| Row layout (two lines, live edge, focused wash and outline) | Pass | `app/sidebar.rs::the_sidebar_lists_agents_under_its_top_row`; [`06-agents-list--04-look-at-done`](images/parity/06-agents-list--04-look-at-done.jpg) |
| Status marks | Pass | same; spinner, "!" pill, dot and ring match in every e2e sidebar |
| Row title | Pass | `model/agents.rs::titles_prefer_names_then_live_titles_and_never_a_bare_tab_number` |
| Times survive restarts, 30 s refresh | Pass | `model/agents.rs::times_are_stamped_when_the_state_changes_and_survive_in_the_tracker`; `agents.json` in the home; `agents::REFRESH_EVERY` |
| Sub-agents nest, peek on click, fold | Pass | `app/sidebar.rs::a_row_click_focuses_its_agent_and_a_sub_agent_click_peeks`, `::the_icon_or_chevron_folds_sub_agents_and_headers_never_fold` |
| Pi orchestrator and worker nesting | Pass | `model/agents.rs::pi_workers_nest_under_their_orchestrator_while_it_or_a_worker_is_focused` |
| Jump labels on every row | Pass | `model/agents.rs::rows_past_thirty_six_get_two_letter_labels`; [`07-agent-navigation--05-jump-bar`](images/parity/07-agent-navigation--05-jump-bar.jpg) |
| Published order | Pass | `app/sidebar.rs::the_published_order_follows_the_rows`, `model/agents.rs::the_published_order_signs_its_inputs` |
| Machine filter applied | Pass | `model/agents.rs::hidden_machines_leave_the_list_and_say_so_when_nothing_is_left`, `::remote_agents_name_their_machine_and_hide_with_it` |
| Empty states | Pass | `app/sidebar.rs::with_no_agents_the_list_says_how_they_show_up` |
| Click a row focuses its pane | Pass | `app/sidebar.rs::a_row_click_focuses_its_agent_and_a_sub_agent_click_peeks` |
| Old local pins carried over | Intentional | Those pins live in Herdr app's mod storage, which herdr-frontend never reads. Pins are the core's `herdr.pinned` label |
| Icons in the foreground | Fixed | see §2 |
| Status change moves the row | Pass | `app/sidebar.rs::a_status_change_moves_the_row`; [`06-agents-list--02-idle-starts-working`](images/parity/06-agents-list--02-idle-starts-working.jpg) |

## §7 Agent navigation and jump

| Item | Verdict | Evidence |
| --- | --- | --- |
| Next/previous agent in the published order | Pass | `model/modes.rs::agents_step_in_list_order_and_rows_select_by_number`, `::a_sub_agent_in_a_pane_steps_from_its_agent`; [`07-agent-navigation--01-next-agent`](images/parity/07-agent-navigation--01-next-agent.jpg). From a terminal that isn't an agent it goes to the first agent (Herdr app lands on the second; `docs/sidebar.md`) |
| Numbered agents `ctrl-g shift-1…0` | Pass | same; [`07-agent-navigation--04-agent-2`](images/parity/07-agent-navigation--04-agent-2.jpg) |
| Agent jump `ctrl-;` | Pass | `app/agent_jump.rs::the_bar_hangs_at_the_top_and_a_label_focuses_its_row`, `::esc_ctrl_semicolon_and_a_press_outside_leave_without_a_jump`, `::a_hidden_sidebar_shows_for_the_jump_and_hides_after`; [`07-agent-navigation--08-jump-bar-shows-the-sidebar`](images/parity/07-agent-navigation--08-jump-bar-shows-the-sidebar.jpg), [`07-agent-navigation--09-jump-to-row-1-the-sidebar-hides-again`](images/parity/07-agent-navigation--09-jump-to-row-1-the-sidebar-hides-again.jpg) |
| `agent.focus` shows sub-agents | Pass | `model/peek.rs::jumping_to_an_agent_without_a_pane_peeks_it` |

## §8 Agent status, attention and toasts

| Item | Verdict | Evidence |
| --- | --- | --- |
| Attention rule | Pass | `model/attention.rs::an_alert_s_toast_names_the_agent_and_where_it_is_and_goes_there`, `model/modes.rs::attention_alerts_brings_settled_tabs_back_and_looking_marks_done_seen`; [`08-attention--03-explore-needs-you`](images/parity/08-attention--03-explore-needs-you.jpg) (a sub-agent alerts in neither) |
| What an alert does: toast "<name> needs you" / "finished", "<space> / <tab>", click focuses | Pass | `app/attention.rs::an_agent_elsewhere_alerts_with_a_toast_that_goes_to_it`, `::an_agent_on_screen_alerts_without_a_toast`; [`08-attention--02-pi-finishes`](images/parity/08-attention--02-pi-finishes.jpg), [`08-attention--04-done-needs-you`](images/parity/08-attention--04-done-needs-you.jpg). Never a sound, never a native notification (`AGENTS.md`) |
| Done becomes idle on any client | Pass | `model/attention.rs::done_turns_idle_when_another_client_looks`; [`08-attention--05-go-to-pi`](images/parity/08-attention--05-go-to-pi.jpg) |
| Toasts: 340 × 72, bottom right 16 in, 8 gaps, 5 s / 8 s, level dot, click runs and dismisses | Pass | `app/keymap.rs::unbuilt_actions_show_a_not_yet_toast_that_goes`; the cards measure 358 window points wide in both apps' screenshots (340 × zoom) |
| The login card: "<m> needs a login: run", the command and Copy | Fixed | Missing before. A new space, tab or split on a machine that needs a login now shows the card, and Copy copies the command and leaves the card up: `app/machines.rs::a_new_space_on_a_machine_that_needs_a_login_says_what_to_run_with_copy` |
| Notes about agents on hidden machines close at once | Pass | `app/attention.rs::notes_about_agents_on_hidden_machines_close_at_once` |
| Other toasts: Neovim navigation failed, copy mode without a terminal, terminal failed to start | Fixed | "Terminal failed to start" was missing: a terminal this window started that exits non-zero within 2 s now says so with its last lines (`model/layout.rs::a_terminal_started_here_that_fails_at_once_says_so_with_its_last_lines`, `src/closed.rs::a_quick_failing_exit_is_a_launch_failure`). The others: `model/layout.rs::nvim_that_cant_run_says_so_and_nothing_moves`, `model/modes.rs::copy_mode_needs_a_focused_terminal_and_ends_when_focus_leaves_it` |
| What "Terminal failed to start" says | Intentional | "Exit code 127 · <its last lines>" on one line. Herdr app puts the lines below "Exit code 127", and `eli-toast` shows a body's first line only, so Eli's card reads just "Exit code 127". The program's own last words are what says why it failed |
| Other toasts: edits moved to `<name>-edited`, layout needs missing mods, core update failed, `set-shared` misuse | Intentional | herdr-frontend has no layout or mod files, doesn't update herdr-core, and serves no `set-shared` |
| Other toasts: dead space closed | Pass | "Closed space <name>", "Its folder no longer exists: <folder>": `model/dead_spaces.rs::a_space_whose_folder_was_deleted_closes_on_the_second_sweep` |

## §9 Machines

| Item | Verdict | Evidence |
| --- | --- | --- |
| Remote spaces | Pass | `app/remote_machine.rs::another_machines_layout_terminals_and_line_come_through_the_local_core` (a real second isolated core); `app/machines.rs::another_machines_spaces_and_agents_show_beside_this_ones` |
| Which machine is shown (the tab bar's end) | Pass | same |
| Agents' machine on line 2 | Pass | `model/agents.rs::remote_agents_name_their_machine_and_hide_with_it` |
| Machine filter popup | Pass | `app/machines.rs::the_machine_filter_lists_machines_and_space_hides_one_in_the_shared_state`, `::a_hidden_machine_known_only_from_the_list_stays_until_shown`; [`09-machines--01-machine-filter`](images/parity/09-machines--01-machine-filter.jpg), [`09-machines--02-hide-this-computer`](images/parity/09-machines--02-hide-this-computer.jpg). The header button opens it where `ctrl-g /` does (Herdr app: a 280 × 260 popup under the button); `docs/machines.md` |
| Remote-aware features | Pass | Neovim: `model/layout.rs::a_remote_neovim_gets_its_keys_on_its_own_machine`; links: `model/links.rs::a_remote_pane_looks_up_and_opens_there_with_no_opener`; new space: `model/popups.rs::on_another_machine_the_folders_and_home_come_from_there`; review: `model/peek.rs::a_space_on_another_machine_uses_that_machines_review`; PR finder and toasts honor the filter (§6, §8) |
| Machine management | Intentional | A Machines dialog and an add dialog instead of Settings → Machines: `app/machines.rs::the_machines_dialog_shows_each_line_and_what_a_machine_needs`, `::a_machines_menu_connects_copies_renames_and_removes`, `::adding_a_machine_lists_the_ssh_hosts_not_added_yet_and_adds_the_one_picked`; [`09-machines--05-machines-dialog`](images/parity/09-machines--05-machines-dialog.jpg). `docs/machines.md` lists the differences |

## §10 Pickers, palette, prompt, confirm, menus, help

| Item | Verdict | Evidence |
| --- | --- | --- |
| Tab/agent picker `cmd-p` | Pass | `app/popups.rs::the_picker_goes_to_tabs_closes_them_and_unfolds_quiet_spaces`, `model/popups.rs::the_picker_puts_busy_spaces_first_and_folds_quiet_ones`; [`10-pickers--01-tab-picker`](images/parity/10-pickers--01-tab-picker.jpg). It filters with Eli's matching, not Herdr app's substring filter (`docs/popups.md`) |
| The picker's working spinner | Fixed | It was a static ◐; the shipped picker spins one. Same test |
| Command palette | Pass | `app/popups.rs::the_palette_filters_runs_a_command_and_remembers_it`, `model/popups.rs::the_palette_lists_recent_commands_first_then_every_group`; [`10-pickers--03-command-palette`](images/parity/10-pickers--03-command-palette.jpg), [`10-pickers--04-type-in-the-palette`](images/parity/10-pickers--04-type-in-the-palette.jpg) |
| The palette's key hints and entries | Intentional | Each row, like each menu item, shows the command's prefix key when it has one (New tab: `ctrl-g t`, Rename tab: `ctrl-g r`), else its other key (`model/help.rs::hints_are_the_prefix_key_else_another`, `model/popups.rs::the_palette_lists_recent_commands_first_then_every_group`). Herdr app shows the alphabetically first one (`cmd-t`), a side effect of the map its keys come from. The palette also lists Eli's own commands (the jump list, copy mode, agent jump, review), which Herdr app's palette doesn't list, and has no "reset to the default keys" command |
| Lists scroll, the cursor stays | Intentional | The palette and pickers are one flat scroll list; the wheel scrolls it and never moves the cursor, a list that fits doesn't scroll, and a moved cursor's row is revealed (`app/popups.rs::the_palette_is_one_flat_list_the_wheel_scrolls_without_moving_the_cursor`, `::a_list_that_fits_doesnt_scroll`). Eli's space-picker, new-space and PR-finder mods drew a window of rows with "↑ N more" / "↓ N more" and no wheel; herdr-frontend scrolls instead |
| Prompt dialog | Pass | `app/popups.rs::the_prompt_cancels_with_esc_and_saves_with_its_button`, `model/popups.rs::the_prompt_saves_trimmed_names_and_the_confirm_closes_the_space`; [`04-tabs--07-rename-prompt`](images/parity/04-tabs--07-rename-prompt.jpg) |
| Confirm dialog | Pass | `app/popups.rs::closing_a_space_asks_and_cancel_or_confirm_answers` |
| Tab menu | Pass | `model/tabs.rs::right_click_opens_the_tab_menu_with_its_items_and_key_hints`, `::the_tab_menu_runs_rename_new_tab_and_close_on_its_tab` |
| Terminal menu | Pass | `app/popups.rs::menus_open_where_the_right_click_was_and_run_their_items`, `model/popups.rs::the_terminal_menu_splits_zooms_routes_right_clicks_and_sends_sub_agents_back`. Inside the peek, without the pane items (043, §15): `app/peek.rs::a_right_click_in_the_peek_opens_the_terminal_menu_without_pane_items`, `model/popups.rs::the_peeks_terminal_menu_leaves_the_pane_items_out` |
| Keys help | Pass, Intentional, Fixed | `app/keymap.rs::the_keys_help_lists_the_bindings_filters_and_closes`; [`10-pickers--06-keys-help`](images/parity/10-pickers--06-keys-help.jpg). The header now reads "Keybinds" and Esc over the field, as the help mod's does (fixed). Rows show each command's title with its keys instead of raw command words, which fixes the limit the inventory notes (intentional) |
| Welcome / What's new / Feedback frames | Intentional | Herdr app's product frames (its onboarding, release notes and feedback channel). herdr-frontend has none |

## §11 New space

| Item | Verdict | Evidence |
| --- | --- | --- |
| New space in a picked folder (`cmd-n`, `ctrl-g ctrl-s`) | Pass | `app/popups.rs::the_folder_picker_lists_where_you_are_then_zoxide_then_repos_and_makes_the_space`, `::a_typed_path_leads_and_nothing_matching_says_where_enter_goes`; `model/popups.rs::enter_while_the_folders_are_loading_waits_when_nothing_matches`; [`11-new-space--01-new-space-picker`](images/parity/11-new-space--01-new-space-picker.jpg) (the same folders, in the same order), [`11-new-space--02-type-a-path`](images/parity/11-new-space--02-type-a-path.jpg) |

## §12 PR finder

| Item | Verdict | Evidence |
| --- | --- | --- |
| Which agent owns this PR (`ctrl-g f`) | Pass | `app/popups.rs::the_pr_finder_lists_agents_watching_prs_filters_and_focuses_one`, `::the_pr_finder_says_when_no_agent_watches_a_pr`; `src/prs.rs`; [`12-pr-finder--02-type-a-pr-number`](images/parity/12-pr-finder--02-type-a-pr-number.jpg) |

## §13 Review view

| Item | Verdict | Evidence |
| --- | --- | --- |
| `cmd-r` focuses this machine's review, starts one when none runs, goes back | Pass | `model/peek.rs::cmd_r_goes_to_this_machines_review_and_back`, `::without_one_cmd_r_starts_a_labelled_review_tab_in_the_shown_space`, `::a_review_started_on_another_machines_space_runs_there`; [`13-review--01-review`](images/parity/13-review--01-review.jpg) (the real `review` running in both, its tab hidden), [`13-review--02-back`](images/parity/13-review--02-back.jpg) |

§14 (Files and Changes panels) is shipped but unused, so there's nothing to check: there are no
panel buttons ([`14-panels--01-a-shell-tab`](images/parity/14-panels--01-a-shell-tab.jpg)).

## §15 Peek

| Item | Verdict | Evidence |
| --- | --- | --- |
| Peek a terminal: 80% × 80% over a dimmed window, the 34 px bar, go to, keep, ✕, Esc to the terminal | Pass | `app/peek.rs::a_sub_agents_row_opens_the_peek_with_its_bar_and_terminal`, `::keys_go_to_the_peeked_terminal_and_esc_too`, `::keep_gives_it_a_tab_and_focuses_it`, `::go_to_shows_a_peeked_terminals_own_pane`, `::cmd_click_on_a_live_terminals_address_peeks_it`; [`15-peek--01-peek-explore`](images/parity/15-peek--01-peek-explore.jpg) (the window behind reads (19, 19, 19) in Herdr app and (20, 20, 20) here: dimmed 35% in both) |
| Find bar, terminal menu and the Cmd-hover underline inside the peek | Pass (043) | Find: `app/peek.rs::cmd_f_in_the_peek_opens_its_find_bar`, `::a_peeked_terminals_find_bar_is_over_the_peek_not_its_pane`; e2e `15-peek` opens the bar and finds a word in both apps. Menu without the pane items: `app/peek.rs::a_right_click_in_the_peek_opens_the_terminal_menu_without_pane_items`, `model/popups.rs::the_peeks_terminal_menu_leaves_the_pane_items_out`. Cmd-hover underlines a live terminal's address ahead of a link, in panes and in the peek: `app/peek.rs::cmd_hover_underlines_a_live_terminals_address`, `terminal/tests/vt.rs::a_hovered_terminal_address_is_underlined_before_a_link` |

## §16 Copy mode

| Item | Verdict | Evidence |
| --- | --- | --- |
| Keyboard copy mode (`ctrl-g [`): bar, motions, selections, yank, leave | Pass | `app/copy_mode.rs` (6 tests), `copy-mode/tests/motions.rs` (21 tests), `terminal/tests/copy.rs`; [`16-copy-mode--03-copy-mode`](images/parity/16-copy-mode--03-copy-mode.jpg) to [`16-copy-mode--11-leave`](images/parity/16-copy-mode--11-leave.jpg) |
| The mod's limits (no word motions, no search, `^`/`$` tricks, no history on the alternate screen) | Intentional | Lifted: herdr-frontend owns the buffer, so copy mode has Vim's words, search, marks, text objects and blocks (`docs/keys.md` "Modes") |

## §17 Terminal behavior

| Item | Verdict | Evidence |
| --- | --- | --- |
| Search (`cmd-f`) over screen and scrollback, per-terminal query | Pass | `app/scrollback.rs::cmd_f_finds_in_the_screen_and_scrollback`, `::each_terminal_keeps_its_own_query`; [`17-terminal--05-find-a-word-in-the-screen-and-scrollback`](images/parity/17-terminal--05-find-a-word-in-the-screen-and-scrollback.jpg) |
| Copy/paste: `cmd-c`, `cmd-v`/`ctrl-v`, files paste as dropped, images, `ctrl-shift-v`, copy on select | Pass | `app/input.rs::cmd_v_pastes_and_cmd_c_copies_the_selection`, `::pastes_follow_bracketed_paste_and_files_paste_as_their_paths`, `::an_image_alone_is_ctrl_v_for_a_local_program`, `::a_remote_terminal_gets_files_copied_beside_it_first`, `::dragging_selects_and_letting_go_copies_while_the_program_leaves_the_mouse` |
| Editing keys sent as bytes | Pass | `app/input.rs::the_terminal_key_sends_go_as_their_bytes` |
| Mouse: drag past the edge scrolls, Shift+click extends, tracked mouse, right-click per label, no scrollbars | Pass | `terminal/tests/vt.rs::a_drag_held_past_the_top_scrolls_back_and_keeps_selecting`, `::shift_click_moves_the_end_of_the_selection`; `app/input.rs::a_program_tracking_the_mouse_hears_clicks_drags_and_the_wheel`, `::right_clicks_go_to_the_program_only_when_its_terminal_says_so` |
| Links: Cmd-click URLs, paths through nvim-links, addresses peek | Pass | `app/scrollback.rs::cmd_hover_underlines_links_and_cmd_click_opens_urls`, `::cmd_click_on_a_source_path_opens_it_in_neovim_beside_the_pane`; `app/peek.rs::cmd_click_on_a_live_terminals_address_peeks_it` |
| File links open in Neovim (local and remote) | Pass | `model/links.rs` (6 tests), `src/links.rs` |
| OSC 52 writes (≤ 1 MiB, reads never), `minimum_contrast`, drag and drop (remote: copied there) | Pass | `terminal/tests/vt.rs::clipboard_writes_are_capped`, `::clipboard_writes_can_be_denied`; `view/terminal/rows.rs::minimum_contrast_whitens_or_blackens_dim_text_but_never_graphics`; `app/input.rs::a_file_dropped_on_a_pane_types_its_path`, `::a_remote_terminal_gets_files_copied_beside_it_first` |
| Kitty graphics | Pass, Intentional | Drawn as Ghostty draws them (ticket 042, `docs/research/terminal-rendering.md` §2 "Kitty graphics"): `terminal/tests/images.rs` (`kitten icat`'s recording: every placement's layer, cell, size and crop, formats, scrolling, deletion, render holds, animation), `app/terminal_view.rs::kitty_images_draw_in_their_layers`, `::images_scale_to_the_cell_the_pane_draws`. `just ghostty-diff images` puts every image on Ghostty's pixels. Intentional: pixels show as sRGB, as kitty shows them and as Ghostty intends; Ghostty.app shows them as Display P3 values, so saturated images look more vivid there (`docs/research/renderer-gates.md` "Kitty graphics") |
| Select all (palette) | Pass | `app/scrollback.rs::select_all_takes_the_scrollback_and_copy_on_select_can_be_off`; [`17-terminal--09-select-all`](images/parity/17-terminal--09-select-all.jpg). Ghostty's select all ends at the last cell with text, the prompt row included. Herdr app's selection stops a row short in the screenshot |

## §18 Session continuity and naming

| Item | Verdict | Evidence |
| --- | --- | --- |
| Pi resumes after a reboot: a reopened terminal is the same address with its labels, and 7501/DA aren't answered in a way that breaks Pi | Pass | Panes follow addresses (`app/shell.rs::after_a_dropped_connection_every_pane_reattaches_resumed`). The renderer leaves OSC 7501 queries to herdr-core: no program-status callback is set, so libghostty-vt never answers them. Replies the core confirms go as AUTOMATIC_INPUT (`app/shell.rs::every_pane_answers_queries_as_automatic_input`, `terminal/tests/vt.rs::leaves_queries_it_cannot_answer_unanswered`) |
| Tab names follow Pi session names (`do tab.rename`) | Pass | `model/control.rs::renames_write_trimmed_names_and_empty_goes_back_to_automatic`; [`18-session--02-pi-names-the-tab`](images/parity/18-session--02-pi-names-the-tab.jpg) |
| Kept terminals: new panes are `keep: true`, closing a pane ends its terminal | Pass | `model/layout.rs::a_new_tab_starts_a_labeled_terminal_then_goes_after_the_selected_tab` (`run.keep`), `::closing_a_pane_kills_its_terminal_and_focus_goes_back` |

## §19 Housekeeping state

| Item | Verdict | Evidence |
| --- | --- | --- |
| Mod storage: the Agents list's times and pins, the tab bar's panel memory | Intentional | Times are in `agents.json` in herdr-frontend's home (§6). Pins are core labels. There are no panels to remember |
| Layout clicks persist: the sidebar's shown state and width survive restarts | Pass | `app/sidebar.rs::a_fresh_home_starts_from_the_defaults_and_keeps_its_own_after` (`sidebar.px`, `sidebar.hidden`) |
| `view.json`: space, tab, pane, zoom, settled tabs, recent spaces, seen attention | Pass | `model/modes.rs::the_view_is_saved_when_it_changes_and_restores_where_its_ids_still_are`, `::saved_attention_comes_back_while_the_agent_says_the_same_word`; [`19-housekeeping--03-restart`](images/parity/19-housekeeping--03-restart.jpg) (both reopen on the blocked agent's tab) |

## §22 Keybinding table

Every binding in the table is in herdr-frontend's default key file, which is Eli's `keys.json`
verbatim (`crates/model/src/keys.json`). `model/modes.rs::every_binding_runs_in_every_mode` runs
each one, and `::app_keys_become_app_commands` checks the app commands. Each key's action is
covered by its section above. The keys inside Eli's popups (§22 "Keys inside Eli's popups") are
`model::mode::popup_key` and copy mode's grammar: `model/modes.rs::list_popups_move_wrap_pick_and_close`,
`::the_picker_closes_a_tab_and_the_machine_filter_toggles_without_closing`,
`::agent_jump_focuses_a_row_by_its_label_and_ignores_modified_keys`,
`::elis_copy_mode_escape_drops_the_selection_then_leaves`. The mouse gestures are the §4, §5, §6
and §17 rows. Two keys behave differently:

- `cmd-h` and `cmd-alt-h` hide the app through macOS's app menu, which takes them before the
  window does. The window's own `app.hide` would say "Not yet", but no key or palette entry
  reaches it.
- `cmd-shift-l` stays on `eli` (§1).

## Appendix: Eli's regression checks

Each check in `~/.dotfiles/tests/` and what covers its cases here. The checks that test copies of
mod files staying identical, or key file entries, have nothing to port: herdr-frontend has one
copy of each rule, and its default key file is Eli's.

| Check | Covered by |
| --- | --- |
| `herdr-app-navigation.mjs` (+ `.lua`) | `model/layout.rs::neovim_*`, `::a_labeled_neovim_that_is_gone_loses_its_label_and_the_pane_moves`, `::nvim_that_cant_run_says_so_and_nothing_moves`, `::a_remote_neovim_gets_its_keys_on_its_own_machine`; the numbered-agent fallback sort is unnecessary (the list is built whether or not the sidebar shows, `docs/sidebar.md`). The Lua half is Neovim's own and unchanged: herdr-frontend reads and writes the same labels |
| `herdr-app-jump-list.mjs` | `src/view.rs` jump-list tests (back and forward, Neovim's ordering, the 1 s dwell, closed tabs skipped, 100 entries) |
| `herdr-app-agent-tabs.mjs`, `herdr-app-tab-close-focus.mjs` | `model/tabs.rs`, `model/layout.rs::tab_keys_*`, `::closing_*`; `app/tabs.rs::the_wheel_steps_through_the_shown_tabs` |
| `herdr-app-tab-icons.mjs` | `app/look.rs::tabs_show_their_programs_icon` |
| `herdr-app-session-names.mjs` | `src/agents.rs::pi_session_names_come_from_the_title`, `model/tabs.rs::unnamed_tabs_are_numbered_among_the_shown_tabs_and_pi_tabs_take_their_session`, `::a_suffix_someone_already_used_gets_a_further_one` |
| `herdr-app-agent-jump.mjs` | `src/mode.rs` jump-label tests, `model/agents.rs::rows_past_thirty_six_get_two_letter_labels`; the minute-old stamp has no counterpart (the list reads the open bar directly) |
| `herdr-app-agent-watch.mjs` | `model/agents.rs::pi_workers_nest_under_their_orchestrator_while_it_or_a_worker_is_focused`, `src/agents.rs::pi_watch_labels_are_trusted_only_on_pi` |
| `herdr-app-agent-colors.mjs` | `src/agents.rs` space-color tests |
| `herdr-app-agent-machines.mjs` | `app/machines.rs::the_machine_filter_lists_machines_and_space_hides_one_in_the_shared_state`, `::a_hidden_machine_known_only_from_the_list_stays_until_shown`; `app/attention.rs::notes_about_agents_on_hidden_machines_close_at_once` |
| `herdr-app-space-picker.mjs` | `src/filter.rs` (a port of `match.js`), `model/popups.rs::going_to_a_space_shows_a_terminal_tab_when_its_selected_one_is_an_agents`; branch matching: `model/checkouts.rs::the_space_picker_matches_the_branch_of_the_terminal_a_space_shows_first` |
| `herdr-app-pr-finder.mjs` | `src/prs.rs` (a port, test for test) |
| `herdr-app-new-space.mjs` | `src/folders.rs` (a port, test for test) |
| `herdr-app-copy-mode.mjs` | `copy-mode/tests/motions.rs`, `app/copy_mode.rs`; herdr-frontend's copy mode is a superset of the mod's state machine |
| `herdr-app-nvim-links.mjs` | `src/links.rs`, `model/links.rs` |
| `herdr-app-review-view.mjs` | `model/peek.rs::cmd_r_*`, `::without_one_cmd_r_starts_a_labelled_review_tab_in_the_shown_space` |
| `herdr-app-dead-spaces.mjs` | `model/dead_spaces.rs` (a port, case for case) |
| `hc-pi-resume.sh` | herdr-core and fish, not herdr-frontend (§18) |
| `herdr-font-preparation.py`, `herdr-app-font.swift` | Not ported: the prepared fonts are Herdr app's workaround. `terminal/tests/golden.rs::operator_mono_book`, `::operator_mono_styles_have_the_regular_cell` check the installed faces |

## Terminal rendering against Ghostty

`just ghostty-diff` on this branch (Ghostty.app 1.4.0; `colors`, `boxes`, `nvim` and
`nvim-checklist` in JetBrains Mono 14 pt, `styles` in Eli's Operator Mono Book at 18.7 pt with
its Bold and italics). Backgrounds are identical in every screen, and text sits within a quarter
of a device pixel of Ghostty's with the same ink weight:

| Screen | Cells | Identical | Pixels differing | > 64 of 255 | Mean abs diff | Ink weight (ours / Ghostty) |
| --- | --- | --- | --- | --- | --- | --- |
| colors | 9027 | 8267 | 3.39% | 0.00% | 0.08 | 1.000 |
| boxes | 9027 | 8735 | 0.39% | 0.00% | 0.03 | 1.001 |
| nvim | 9027 | 6545 | 9.16% | 0.00% | 0.35 | 1.000 |
| nvim-checklist | 9027 | 8368 | 2.30% | 0.00% | 0.08 | 1.000 |
| styles | 6048 | 5219 | 1.63% | 0.00% | 0.33 | 0.999 |

The differing pixels are anti-aliasing at glyph edges: under 0.01% of them differ by more than a
quarter of the range, and the text's centroid moves at most 0.21 px
(`docs/research/renderer-gates.md` explains where the rest comes from). Herdr app's heavier text in the e2e screenshots is its prepared font
family (§2), not a renderer difference to match.

The `images` screen (ticket 042, `kitten icat` in each layer) places every image on the same
pixels as Ghostty. Its colors differ on purpose: Ghostty shows image pixels as Display P3 values,
herdr-frontend as sRGB (`docs/research/renderer-gates.md` "Kitty graphics").
