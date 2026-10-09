//! Input.
use crate::app::{
    keybinds, App, Arc, BindKey, CursorGrabMode, DemoViewMode, ElementState, FrontendPage, HashSet,
    Instant, KeyCode, KeyEvent, OverlayMode, RenderCommand,
};

impl App {
    pub(in crate::app) fn set_capture(&mut self, captured: bool) {
        if !captured {
            self.noclip_primary_down = false;
            self.noclip_alt_down = false;
        }
        if self.captured == captured {
            return;
        }
        let Some(window) = &self.window else {
            return;
        };
        if captured && self.overlay == OverlayMode::None {
            let grab_mode = if window.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
                Some("locked")
            } else if window.set_cursor_grab(CursorGrabMode::Confined).is_ok() {
                Some("confined")
            } else {
                None
            };
            if let Some(grab_mode) = grab_mode {
                window.set_cursor_visible(false);
                self.captured = true;
                self.mouse_grab_mode = grab_mode;
            } else {
                self.captured = false;
                self.mouse_grab_mode = "none";
            }
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            self.captured = false;
            self.mouse_grab_mode = "none";
        }
    }

    pub(in crate::app) fn sync_profile_preview_mode(&mut self, active: bool) {
        if self.profile_preview_active == active {
            return;
        }
        // Only commit a selected model when the entire Profile menu closes,
        // not while clicking tiles, switching Profile sections, or pressing APPLY.
        // This route is shared by ESC, Back, top-level tabs, and uipage.
        if !active {
            if let Some(model) = self.profile_model_staged.take() {
                if !model.eq_ignore_ascii_case(&self.solo_client_info.model_cvar()) {
                    if let Err(error) = self.set_console_cvar("model", &model) {
                        self.console_status = error;
                    }
                }
            }
        }
        self.profile_preview_active = active;
        if active {
            self.profile_name_input = self.network.name.clone();
            self.profile_preview_started = Instant::now();
        }
        self.profile_force_preview_pending = None;
        self.profile_force_preview_active = None;
        self.profile_force_preview_started = Instant::now();
        self.profile_preview_key = None;
        self.asset_preview_viewport_key = None;
        self.render_command(RenderCommand::SetAssetPreviewMode(active));
        self.render_command(RenderCommand::SetAssetPreviewViewport(None));
        if !active {
            self.profile_dynamic_models = Arc::new(Vec::new());
        }
    }

    pub(in crate::app) fn set_overlay(&mut self, overlay: OverlayMode) {
        let leaving_console =
            self.overlay == OverlayMode::Console && overlay != OverlayMode::Console;
        let toggling_console = self.overlay != overlay
            && (self.overlay == OverlayMode::Console || overlay == OverlayMode::Console);
        // TaystJK Con_ToggleConsole_f calls Con_ClearNotify before changing the
        // console catcher, so console command/output rows do not leak back onto
        // gameplay after opening or closing the solid console.
        if toggling_console {
            self.console_notify.clear();
            self.last_console_notify_refresh = Instant::now();
        }
        self.overlay = overlay;
        self.sync_profile_preview_mode(
            self.overlay == OverlayMode::Game
                && if self.front_end {
                    self.frontend_page == FrontendPage::Profile
                } else {
                    self.menu_selected == 2
                },
        );
        self.egui_repaint_requested = true;
        if leaving_console {
            self.console_input.clear();
            self.console_cursor = 0;
            self.console_history_index = None;
        }
        if overlay != OverlayMode::Console {
            self.console_search_open = false;
        }
        self.last_mouse_motion_at = None;
        if let Some(player) = &mut self.local_server {
            player.pause();
        }
        if overlay != OverlayMode::None {
            // Equivalent to TaystJK Key_ClearStates: menus/console must not
            // leave a gameplay kbutton latched if its physical release is
            // consumed by UI input routing.
            self.release_all_pressed_bindings();
            self.keys.clear();
            self.movement_keys.clear();
            self.mouse_buttons_down.clear();
            self.noclip_primary_down = false;
            self.noclip_alt_down = false;
            self.set_capture(false);
        } else if self.demo_playback_active() {
            self.sync_demo_camera_capture();
        } else {
            self.set_capture(true);
        }
        self.publish_ui();
    }

    pub(in crate::app) fn controls_page_active(&self) -> bool {
        self.overlay == OverlayMode::Game
            && if self.front_end {
                self.frontend_page == FrontendPage::Controls
            } else {
                self.menu_selected == 3
            }
    }

    pub(in crate::app) fn overlay_after_console(&self) -> OverlayMode {
        if self.front_end && self.overlay_before_console == OverlayMode::None {
            OverlayMode::Game
        } else {
            self.overlay_before_console
        }
    }

    pub(in crate::app) fn frontend_back(&mut self) {
        if !self.front_end {
            return;
        }
        if self.overlay == OverlayMode::Video {
            self.frontend_page = FrontendPage::Main;
            self.set_overlay(OverlayMode::Game);
            return;
        }
        if self.frontend_page == FrontendPage::Profile {
            self.sync_profile_preview_mode(false);
        } else if self.frontend_page == FrontendPage::AssetViewer {
            self.render_command(RenderCommand::SetAssetPreviewMode(false));
            self.render_command(RenderCommand::SetAssetPreviewViewport(None));
            self.asset_preview_viewport_key = None;
            self.clear_asset_preview_runtime();
        }
        self.frontend_page = match self.frontend_page {
            FrontendPage::Main => FrontendPage::Main,
            FrontendPage::Play
            | FrontendPage::Profile
            | FrontendPage::Controls
            | FrontendPage::ChatLogs
            | FrontendPage::DeveloperTools
            | FrontendPage::Screenshots => FrontendPage::Main,
            FrontendPage::ServerBrowser | FrontendPage::SoloGame | FrontendPage::PlayDemo => {
                FrontendPage::Play
            }
            FrontendPage::AssetViewer | FrontendPage::MapViewer => FrontendPage::DeveloperTools,
        };
        self.egui_repaint_requested = true;
        self.publish_ui();
    }

    pub(in crate::app) fn mark_config_dirty(&mut self) {
        self.config_dirty = true;
        self.last_config_write = Instant::now();
    }

    pub(in crate::app) fn refresh_bound_state(&mut self) {
        let mut movement_keys = HashSet::with_capacity(8);
        for command in &self.live_buttons {
            match command.as_str() {
                "+forward" => {
                    movement_keys.insert(KeyCode::KeyW);
                }
                "+back" => {
                    movement_keys.insert(KeyCode::KeyS);
                }
                "+moveleft" => {
                    movement_keys.insert(KeyCode::KeyA);
                }
                "+moveright" => {
                    movement_keys.insert(KeyCode::KeyD);
                }
                "+moveup" => {
                    movement_keys.insert(KeyCode::Space);
                }
                "+movedown" => {
                    movement_keys.insert(KeyCode::ControlLeft);
                }
                "+speed" => {
                    movement_keys.insert(KeyCode::ShiftLeft);
                }
                _ => {}
            }
        }
        self.movement_keys = movement_keys;
        self.noclip_primary_down = self.live_buttons.contains("+attack");
        self.noclip_alt_down = self.live_buttons.contains("+altattack");
        self.scores_showing = self.live_buttons.contains("+scores");
    }

    fn sync_button_transitions(
        &mut self,
        was_scores_showing: bool,
        was_demo_ui_modifier: bool,
        was_zoomed: bool,
    ) {
        let is_zoomed = self.live_buttons.contains("+zoom");
        if is_zoomed != was_zoomed {
            // TaystJK CG_ZoomDown_f / CG_ZoomUp_f record cg.zoomTime when the
            // held +zoom state changes; keep the same transition epoch here.
            self.japro_zoom_transition_at = Some(Instant::now());
        }
        self.refresh_bound_state();
        if self.demo_playback_active()
            && was_demo_ui_modifier != self.live_buttons.contains("+speed")
        {
            self.sync_demo_camera_capture();
        }
        if self.scores_showing && !was_scores_showing {
            // TaystJK's CG_ScoresDown_f does not send `score` or clear the
            // cached board during demo playback. Rebuild from the demo index;
            // live play keeps the normal ~2 second score request cadence.
            if !self.refresh_demo_scoreboard(Instant::now()) {
                self.request_scores_if_due(true);
            }
        }
        if self.scores_showing != was_scores_showing {
            self.publish_ui();
        }
    }

    /// Port of TaystJK/OpenJK IN_KeyDown/IN_KeyUp ownership semantics. The
    /// command buffer supplies an internal `@bind:<key>` source for physical
    /// binds; commands typed manually have no source and `-foo` clears all
    /// sources, matching IN_KeyUp's console-un-sticking behavior.
    pub(in crate::app) fn execute_input_button_command(&mut self, command: &str) -> bool {
        let mut words = command.split_whitespace();
        let Some(raw_verb) = words.next() else {
            return false;
        };
        let verb = raw_verb.to_ascii_lowercase();
        let Some(stem) = verb.strip_prefix('+').or_else(|| verb.strip_prefix('-')) else {
            return false;
        };
        if !keybinds::is_supported_button_stem(stem) {
            return false;
        }
        let is_down = verb.starts_with('+');
        let plus_verb = format!("+{stem}");
        let source = words
            .find(|word| word.starts_with("@bind:"))
            .map(str::to_owned);

        // Demo mouse attack bindings are POV controls in this client. Keep the
        // behavior, but execute it at Cbuf time so `wait` also orders it.
        if self.demo_playback_active() && is_down {
            let follow_direction = if plus_verb == "+attack" {
                Some(1)
            } else if plus_verb == "+altattack" {
                Some(-1)
            } else {
                None
            };
            if let Some(direction) = follow_direction {
                if !self.live_buttons.contains("+speed") {
                    self.cycle_demo_follow(direction);
                }
                return true;
            }
        }

        let was_scores_showing = self.scores_showing;
        let was_demo_ui_modifier = self.live_buttons.contains("+speed");
        let was_zoomed = self.live_buttons.contains("+zoom");
        let was_active = self.live_buttons.contains(&plus_verb);

        if is_down {
            let source = source.unwrap_or_else(|| "@manual".to_owned());
            let inserted = self
                .live_button_sources
                .entry(plus_verb.clone())
                .or_default()
                .insert(source);
            if !inserted {
                return true;
            }
            if !was_active {
                self.live_buttons.insert(plus_verb.clone());
                // IN_KeyDown wasPressed: a tap shorter than a frame still fires.
                self.live_input.note_pressed(&plus_verb);
                if let Some(player) = &mut self.local_server {
                    if let Some(key) = keybinds::canonical_movement_key(&plus_verb) {
                        player.note_movement_key_press(key);
                    }
                }
                if plus_verb == "+moveup"
                    && self.demo_playback_active()
                    && self.demo_view_mode != DemoViewMode::Free
                {
                    self.enter_demo_free_view();
                }
            }
        } else if let Some(source) = source {
            let mut remove_entry = false;
            if let Some(sources) = self.live_button_sources.get_mut(&plus_verb) {
                sources.remove(&source);
                remove_entry = sources.is_empty();
            }
            if remove_entry {
                self.live_button_sources.remove(&plus_verb);
                self.live_buttons.remove(&plus_verb);
            }
        } else {
            // TaystJK IN_KeyUp with no key argument is an explicit unstick.
            self.live_button_sources.remove(&plus_verb);
            self.live_buttons.remove(&plus_verb);
        }

        self.sync_button_transitions(was_scores_showing, was_demo_ui_modifier, was_zoomed);
        true
    }

    /// Whether `code` is currently bound to the `trace` command, so
    /// `route_egui_window_event` can let it fall through to the raw keybind
    /// dispatch instead of being swallowed as "a menu is open" input while
    /// the Trace popup is up — the same key needs to close it again.
    pub(in crate::app) fn key_bound_to_trace(&self, code: KeyCode) -> bool {
        self.bindings
            .get_resolved(
                keybinds::bind_key_for_code(code),
                self.spectator_binding_context(),
            )
            .is_some_and(|binding| keybinds::binding_contains_command(binding, "trace"))
    }

    pub(in crate::app) fn execute_binding_press(&mut self, key: BindKey) {
        self.execute_binding_press_from(key, key);
    }

    pub(in crate::app) fn execute_binding_press_from(
        &mut self,
        key: BindKey,
        source_key: BindKey,
    ) {
        let spectator_context = self.spectator_binding_context();
        let Some(binding) = self
            .bindings
            .get_resolved(key, spectator_context)
            .map(str::to_owned)
        else {
            return;
        };
        // Binding lookup intentionally normalizes left/right modifiers, but a
        // kbutton source must identify the physical key. Use Debug for keyboard
        // codes so Left/Right Ctrl/Shift/Alt remain independent sources.
        let source = match source_key {
            BindKey::Keyboard(code) => format!("@bind:key:{code:?}"),
            _ => format!("@bind:{}", keybinds::key_name(source_key)),
        };
        let mut buffered = Vec::new();
        for command in keybinds::split_binding_commands(&binding) {
            let verb = command.split_whitespace().next().unwrap_or("");
            if verb.starts_with('+') {
                buffered.push(format!("{command} {source}"));
            } else {
                buffered.push(command.to_owned());
            }
        }
        // TaystJK CL_ParseBinding appends the complete bind to Cbuf, including
        // +commands. That ordering is what makes `+speed; wait 2; +movedown`
        // activate crouch only after the wait has elapsed.
        self.append_console_commands(buffered);
    }

    pub(in crate::app) fn execute_binding_release(&mut self, key: BindKey) {
        self.execute_binding_release_from(key, key);
    }

    pub(in crate::app) fn execute_binding_release_from(
        &mut self,
        key: BindKey,
        source_key: BindKey,
    ) {
        let spectator_context = self.spectator_binding_context();
        let Some(binding) = self
            .bindings
            .get_resolved(key, spectator_context)
            .map(str::to_owned)
        else {
            return;
        };
        let source = match source_key {
            BindKey::Keyboard(code) => format!("@bind:key:{code:?}"),
            _ => format!("@bind:{}", keybinds::key_name(source_key)),
        };
        let releases = keybinds::split_binding_commands(&binding)
            .into_iter()
            .filter_map(|command| {
                let verb = command.split_whitespace().next().unwrap_or("");
                verb.strip_prefix('+')
                    .map(|stem| format!("-{stem} {source}"))
            })
            .collect();
        self.append_console_commands(releases);
    }

    pub(in crate::app) fn execute_binding_tap(&mut self, key: BindKey) {
        self.execute_binding_press(key);
        self.execute_binding_release(key);
    }

    pub(in crate::app) fn release_all_pressed_bindings(&mut self) {
        let keyboard: Vec<_> = self.keys.iter().copied().collect();
        let mouse: Vec<_> = self.mouse_buttons_down.iter().copied().collect();
        for code in keyboard {
            self.execute_binding_release_from(
                keybinds::bind_key_for_code(code),
                BindKey::Keyboard(code),
            );
        }
        for button in mouse {
            self.execute_binding_release(BindKey::Mouse(button));
        }
    }

    pub(in crate::app) fn bind_control_key(&mut self, key: BindKey) {
        let Some(action) = keybinds::control_action(self.controls_selected) else {
            return;
        };
        let command = action.command;
        if keybinds::is_spectator_selection(self.controls_selected) {
            self.bindings.unbind_spectator_command(command);
            self.bindings.set_spectator(key, command);
        } else {
            self.bindings.unbind_command(command);
            self.bindings.set(key, command);
        }
        self.controls_waiting_for_key = false;
        self.refresh_bound_state();
        self.mark_config_dirty();
        self.console_status = format!("BOUND {} TO {}", keybinds::key_name(key), command);
        self.publish_ui();
    }

    pub(in crate::app) fn unbind_selected_control(&mut self) {
        if let Some(action) = keybinds::control_action(self.controls_selected) {
            let command = action.command;
            let changed = if keybinds::is_spectator_selection(self.controls_selected) {
                self.bindings.unbind_spectator_command(command)
            } else {
                self.bindings.unbind_command(command)
            };
            if changed > 0 {
                self.refresh_bound_state();
                self.mark_config_dirty();
            }
        }
        self.controls_waiting_for_key = false;
        self.publish_ui();
    }


    /// Keyboard for the Game-side menus. egui owns navigation and activation;
    /// only the two things egui cannot do land here: backing out of the menu,
    /// and capturing a raw key/button while rebinding a control.
    pub(in crate::app) fn handle_game_key(&mut self, event: &KeyEvent, code: KeyCode) {
        if event.state != ElementState::Pressed {
            return;
        }

        if self.controls_page_active() && self.controls_waiting_for_key {
            if event.repeat {
                return;
            }
            if code == KeyCode::Escape {
                self.controls_waiting_for_key = false;
                self.egui_repaint_requested = true;
                self.publish_ui();
            } else {
                self.bind_control_key(keybinds::bind_key_for_code(code));
            }
            return;
        }

        if code != KeyCode::Escape {
            return;
        }

        if self.front_end {
            self.frontend_back();
        } else {
            self.set_overlay(OverlayMode::None);
        }
    }
}
