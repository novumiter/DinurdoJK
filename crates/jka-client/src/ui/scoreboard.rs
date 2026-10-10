//! Scoreboard.
use crate::ui::{
    player_icon_atlas_cell, proportional_text, proportional_text_width, rect, rect_gradient,
    rect_outline, scoreboard_icon_atlas_offset, shared_player_icon_count, text, truncate_jka_text,
    visible_jka_chars, ProportionalFont, UiScoreEntry, UiSnapshot, UiVertex,
};

pub(in crate::ui) fn build_scoreboard(
    out: &mut Vec<UiVertex>,
    ui: &UiSnapshot,
    small_font: Option<&ProportionalFont>,
    w: u32,
    h: u32,
) {
    let Some(board) = &ui.scoreboard else { return };
    let shared_icon_count = shared_player_icon_count(ui);
    let scoreboard_icon_offset = scoreboard_icon_atlas_offset(ui);

    // Renderer-native on purpose: +scores stays a lightweight gameplay overlay
    // and never changes input capture / enters the egui menu stack.
    // Layout is authored in a 1080p-ish screen-space and scales with height so
    // the board keeps the same visual weight from 720p through 4K.
    let s = (h as f32 / 1080.0).clamp(0.72, 2.0);
    let width = (960.0 * s).min((w as f32 - 32.0 * s).max(320.0 * s));
    let x = ((w as f32 - width) * 0.5).max(16.0 * s);
    let y = (h as f32 * 0.065).max(24.0 * s);

    let title_h = 48.0 * s;
    let columns_h = 27.0 * s;
    let row_h = 36.0 * s;
    let bottom_pad = 8.0 * s;
    let overflow_h = if board.entries.is_empty() {
        0.0
    } else {
        20.0 * s
    };
    let section_h = 28.0 * s;

    // Preserve jaPRO/OpenJK ordering, but make team membership structural instead
    // of relying on a tiny per-row colour stripe. Modern competitive scoreboards
    // use strong group headers so red/blue can be parsed at a glance.
    #[derive(Clone, Copy)]
    enum SectionKind {
        Team(i32),
        Spectators,
    }
    enum Line<'a> {
        Entry(&'a UiScoreEntry, usize),
        Section(SectionKind, usize),
    }
    let mut lines: Vec<Line> = Vec::with_capacity(board.entries.len() + 4);
    let team_order: &[i32] = if !board.team_game {
        &[0]
    } else if board.team_scores[0] >= board.team_scores[1] {
        &[1, 2, 0]
    } else {
        &[2, 1, 0]
    };
    let mut players = 0usize;
    for &team in team_order {
        let group_count = board
            .entries
            .iter()
            .filter(|entry| entry.team == team || (!board.team_game && entry.team != 3))
            .count();
        if group_count == 0 {
            if !board.team_game {
                break;
            }
            continue;
        }
        if board.team_game {
            lines.push(Line::Section(SectionKind::Team(team), group_count));
        }
        for (index, entry) in board
            .entries
            .iter()
            .filter(|entry| entry.team == team || (!board.team_game && entry.team != 3))
            .enumerate()
        {
            lines.push(Line::Entry(entry, index));
            players += 1;
        }
        if !board.team_game {
            break;
        }
    }
    let spectators = board.entries.iter().filter(|entry| entry.team == 3).count();
    if spectators > 0 {
        lines.push(Line::Section(SectionKind::Spectators, spectators));
        for (index, entry) in board
            .entries
            .iter()
            .filter(|entry| entry.team == 3)
            .enumerate()
        {
            lines.push(Line::Entry(entry, index));
        }
    }

    let room_for_rows = (h as f32 - y - 18.0 * s - title_h - columns_h - bottom_pad).max(row_h);
    let available = (room_for_rows - overflow_h).max(row_h);
    let line_height = |line: &Line| {
        if matches!(line, Line::Section(..)) {
            section_h
        } else {
            row_h
        }
    };
    let mut shown = 0usize;
    let mut used = 0.0f32;
    for line in &lines {
        if used + line_height(line) > available && shown > 0 {
            break;
        }
        used += line_height(line);
        shown += 1;
    }
    // Never end on a heading with nothing under it.
    if shown > 0 && matches!(lines[shown - 1], Line::Section(..)) {
        used -= section_h;
        shown -= 1;
    }
    let overflow = lines[shown..]
        .iter()
        .filter(|line| matches!(line, Line::Entry(..)))
        .count();
    let footer_h = if overflow > 0 { overflow_h } else { 0.0 };
    let height = title_h + columns_h + used + footer_h + bottom_pad;

    const PANEL_TOP: [f32; 4] = [0.045, 0.055, 0.075, 0.965];
    const PANEL_BOTTOM: [f32; 4] = [0.014, 0.018, 0.028, 0.955];
    const TITLE_LEFT: [f32; 4] = [0.075, 0.105, 0.145, 0.98];
    const TITLE_RIGHT: [f32; 4] = [0.030, 0.040, 0.060, 0.98];
    const ACCENT: [f32; 4] = [0.28, 0.78, 0.94, 1.0];
    const TEXT_BRIGHT: [f32; 4] = [0.94, 0.965, 0.99, 1.0];
    const TEXT: [f32; 4] = [0.82, 0.86, 0.91, 1.0];
    const TEXT_DIM: [f32; 4] = [0.51, 0.58, 0.67, 1.0];

    // A soft, offset shadow plus a low-contrast outer keyline reads much cleaner
    // than the old bright grey rectangle without making the overlay feel like a
    // menu window.
    rect(
        out,
        x - 4.0 * s,
        y + 5.0 * s,
        width + 8.0 * s,
        height + 4.0 * s,
        [0.0, 0.0, 0.0, 0.30],
        w,
        h,
    );
    rect_gradient(
        out,
        x,
        y,
        width,
        height,
        PANEL_TOP,
        PANEL_TOP,
        PANEL_BOTTOM,
        PANEL_BOTTOM,
        w,
        h,
    );
    rect_outline(
        out,
        x,
        y,
        width,
        height,
        (1.0 * s).max(1.0),
        [0.30, 0.38, 0.48, 0.58],
        w,
        h,
    );

    // Header: restrained blue-grey panel with a thin cyan identity line. Team
    // scores (when relevant) live here rather than being mixed into the title.
    rect_gradient(
        out,
        x + 1.0 * s,
        y + 1.0 * s,
        width - 2.0 * s,
        title_h - 1.0 * s,
        TITLE_LEFT,
        TITLE_RIGHT,
        [0.045, 0.060, 0.085, 0.98],
        [0.022, 0.030, 0.046, 0.98],
        w,
        h,
    );
    rect(
        out,
        x + 1.0 * s,
        y + 1.0 * s,
        width - 2.0 * s,
        (2.0 * s).max(1.0),
        ACCENT,
        w,
        h,
    );

    let font_x_scale = w.max(1) as f32 / 640.0;
    let title_font_scale = 0.52;
    let header_font_scale = 0.31;
    let row_font_scale = 0.42;
    let meta_font_scale = 0.32;
    let fallback_title_scale = 1.34 * s;
    let fallback_header_scale = 0.92 * s;
    let fallback_row_scale = 1.06 * s;
    let fallback_meta_scale = 0.88 * s;

    let draw = |out: &mut Vec<UiVertex>,
                value: &str,
                tx: f32,
                baseline_y: f32,
                prop_scale: f32,
                fallback_scale: f32,
                color: [f32; 4]| {
        if let Some(font) = small_font {
            proportional_text(
                out, value, font, tx, baseline_y, prop_scale, color, true, w, h,
            );
        } else {
            // The proportional font is normally resident. Keep the charsgrid
            // fallback so a missing font asset can never hide the scoreboard.
            text(
                out,
                value,
                tx,
                baseline_y - 8.0 * fallback_scale,
                fallback_scale,
                color,
                w,
                h,
            );
        }
    };
    let text_width = |value: &str, prop_scale: f32, fallback_scale: f32| -> f32 {
        if let Some(font) = small_font {
            proportional_text_width(value, font, prop_scale) * font_x_scale
        } else {
            visible_jka_chars(value) as f32 * 6.0 * fallback_scale
        }
    };

    let pad = 18.0 * s;
    let title_baseline = y + title_h * 0.68;
    let title = if board.siege {
        match board.siege_win_team {
            1 => "SIEGE  -  RED TEAM WINS",
            2 => "SIEGE  -  BLUE TEAM WINS",
            _ => "SIEGE",
        }
    } else {
        "SCOREBOARD"
    };
    draw(
        out,
        title,
        x + pad,
        title_baseline,
        title_font_scale,
        fallback_title_scale,
        TEXT_BRIGHT,
    );

    let meta = if board.entries.is_empty() {
        "AWAITING SCORES".to_owned()
    } else if board.team_game {
        format!(
            "^1RED  {}    ^7|    ^4BLUE  {}",
            board.team_scores[0], board.team_scores[1]
        )
    } else {
        format!("{} PLAYER{}", players, if players == 1 { "" } else { "S" })
    };
    let meta_width = text_width(&meta, meta_font_scale, fallback_meta_scale);
    draw(
        out,
        &meta,
        x + width - pad - meta_width,
        y + title_h * 0.66,
        meta_font_scale,
        fallback_meta_scale,
        if board.team_game { TEXT } else { TEXT_DIM },
    );

    let columns_y = y + title_h;
    rect(
        out,
        x + 1.0 * s,
        columns_y,
        width - 2.0 * s,
        columns_h,
        [0.75, 0.82, 0.92, 0.055],
        w,
        h,
    );
    rect(
        out,
        x + 10.0 * s,
        columns_y + columns_h - (1.0 * s).max(1.0),
        width - 20.0 * s,
        (1.0 * s).max(1.0),
        [0.45, 0.56, 0.70, 0.26],
        w,
        h,
    );

    // Numeric columns are right aligned. GT_CTF follows TaystJK's dedicated
    // Score / Captures / Assists / Defends / Ping / Time contract; deaths are
    // deliberately absent in CTF.
    let time_right = x + width - pad;
    let ping_right = time_right - 82.0 * s;
    let (score_right, deaths_right, captures_right, assists_right, defends_right) = if board.ctf {
        let defends = ping_right - 74.0 * s;
        let assists = defends - 74.0 * s;
        let captures = assists - 74.0 * s;
        let score = captures - 76.0 * s;
        (score, 0.0, captures, assists, defends)
    } else {
        let deaths = ping_right - 92.0 * s;
        let score = if board.show_deaths { deaths - 92.0 * s } else { ping_right - 92.0 * s };
        (score, deaths, 0.0, 0.0, 0.0)
    };
    // TaystJK draws READY 64 virtual pixels left of the player name. Reserve
    // that column only while the server's STAT_CLIENTS_READY mask is nonzero.
    let ready_column = if board.ready_clients != 0 { 64.0 * s } else { 0.0 };
    let ready_x = x + pad + 7.0 * s;
    let player_x = ready_x + ready_column + if board.ctf { 18.0 * s } else { 0.0 };
    let header_baseline = columns_y + columns_h * 0.69;

    draw(
        out,
        "PLAYER",
        player_x,
        header_baseline,
        header_font_scale,
        fallback_header_scale,
        TEXT_DIM,
    );
    if board.ctf {
        for (label, right) in [
            ("SCORE", score_right),
            ("CAP", captures_right),
            ("AST", assists_right),
            ("DEF", defends_right),
            ("PING", ping_right),
            ("TIME", time_right),
        ] {
            let tw = text_width(label, header_font_scale, fallback_header_scale);
            draw(out, label, right - tw, header_baseline, header_font_scale, fallback_header_scale, TEXT_DIM);
        }
    } else {
        for (label, right) in [("SCORE", score_right), ("PING", ping_right), ("TIME", time_right)] {
            let tw = text_width(label, header_font_scale, fallback_header_scale);
            draw(out, label, right - tw, header_baseline, header_font_scale, fallback_header_scale, TEXT_DIM);
        }
        if board.show_deaths {
            let tw = text_width("DEATHS", header_font_scale, fallback_header_scale);
            draw(out, "DEATHS", deaths_right - tw, header_baseline, header_font_scale, fallback_header_scale, TEXT_DIM);
        }
    }
    let row_start = columns_y + columns_h;
    let mut cursor = row_start;
    for line in lines.iter().take(shown) {
        let (entry, index) = match *line {
            Line::Section(kind, count) => {
                let (label, accent, fill, score) = match kind {
                    SectionKind::Team(1) => (
                        "RED TEAM",
                        [0.96, 0.25, 0.28, 0.96],
                        [0.34, 0.055, 0.070, 0.34],
                        Some(board.team_scores[0]),
                    ),
                    SectionKind::Team(2) => (
                        "BLUE TEAM",
                        [0.30, 0.55, 1.0, 0.98],
                        [0.055, 0.120, 0.35, 0.36],
                        Some(board.team_scores[1]),
                    ),
                    SectionKind::Team(_) => (
                        "FREE",
                        ACCENT,
                        [0.10, 0.16, 0.20, 0.26],
                        None,
                    ),
                    SectionKind::Spectators => (
                        "SPECTATORS",
                        [0.48, 0.56, 0.66, 0.78],
                        [0.09, 0.11, 0.15, 0.28],
                        None,
                    ),
                };
                rect(
                    out,
                    x + 6.0 * s,
                    cursor + 2.0 * s,
                    width - 12.0 * s,
                    section_h - 3.0 * s,
                    fill,
                    w,
                    h,
                );
                rect(
                    out,
                    x + 6.0 * s,
                    cursor + 2.0 * s,
                    (4.0 * s).max(2.0),
                    section_h - 3.0 * s,
                    accent,
                    w,
                    h,
                );
                rect(
                    out,
                    x + 12.0 * s,
                    cursor + section_h - (1.0 * s).max(1.0),
                    width - 24.0 * s,
                    (1.0 * s).max(1.0),
                    [accent[0], accent[1], accent[2], 0.28],
                    w,
                    h,
                );
                let count_text = format!(
                    "{label}    {count} PLAYER{}",
                    if count == 1 { "" } else { "S" }
                );
                draw(
                    out,
                    &count_text,
                    player_x,
                    cursor + section_h * 0.72,
                    header_font_scale,
                    fallback_header_scale,
                    if matches!(kind, SectionKind::Spectators) { TEXT_DIM } else { TEXT_BRIGHT },
                );
                if let Some(score) = score {
                    let score_text = format!("TEAM SCORE  {score}");
                    let score_w = text_width(&score_text, header_font_scale, fallback_header_scale);
                    draw(
                        out,
                        &score_text,
                        x + width - pad - score_w,
                        cursor + section_h * 0.72,
                        header_font_scale,
                        fallback_header_scale,
                        [accent[0], accent[1], accent[2], 0.96],
                    );
                }
                cursor += section_h;
                continue;
            }
            Line::Entry(entry, index) => (entry, index),
        };
        let row_top = cursor;
        cursor += row_h;
        let row_y = row_top + row_h * 0.68;
        let spectating = entry.team == 3;

        let focused = ui.scoreboard_focus_client == Some(entry.client);
        let row_fill = if focused {
            // TaystJK fills the current `cg.snap->ps.clientNum` score line.
            // Keep that ownership cue stronger than team tint.
            [0.20, 0.55, 0.80, 0.30]
        } else {
            match (entry.team, index % 2) {
                (1, 0) => [0.30, 0.065, 0.075, 0.17],
                (1, _) => [0.24, 0.050, 0.060, 0.11],
                (2, 0) => [0.055, 0.115, 0.31, 0.18],
                (2, _) => [0.045, 0.090, 0.25, 0.12],
                (3, 0) => [0.42, 0.48, 0.58, 0.060],
                (3, _) => [0.32, 0.37, 0.46, 0.032],
                (_, 0) => [0.80, 0.86, 0.96, 0.050],
                _ => [0.55, 0.62, 0.72, 0.022],
            }
        };
        rect(
            out,
            x + 6.0 * s,
            row_top + 2.0 * s,
            width - 12.0 * s,
            row_h - 3.0 * s,
            row_fill,
            w,
            h,
        );
        if focused {
            rect_outline(
                out,
                x + 6.0 * s,
                row_top + 2.0 * s,
                width - 12.0 * s,
                row_h - 3.0 * s,
                (1.0 * s).max(1.0),
                [0.38, 0.82, 1.0, 0.72],
                w,
                h,
            );
        }

        let accent = match entry.team {
            1 => [0.92, 0.22, 0.24, 0.95],
            2 => [0.24, 0.48, 0.96, 0.95],
            3 => [0.50, 0.55, 0.62, 0.72],
            _ => ACCENT,
        };
        rect(
            out,
            x + 6.0 * s,
            row_top + 2.0 * s,
            (3.0 * s).max(2.0),
            row_h - 3.0 * s,
            accent,
            w,
            h,
        );

        if board.ctf {
            let flag = if entry.powerups & (1 << 4) != 0 {
                Some([0.94, 0.20, 0.22, 0.96])
            } else if entry.powerups & (1 << 5) != 0 {
                Some([0.22, 0.46, 0.98, 0.96])
            } else if entry.powerups & (1 << 6) != 0 {
                Some([0.88, 0.88, 0.88, 0.96])
            } else {
                None
            };
            if let Some(flag) = flag {
                let mast_x = player_x - 15.0 * s;
                rect(out, mast_x, row_top + 8.0 * s, (1.5 * s).max(1.0), 20.0 * s, TEXT_DIM, w, h);
                rect(out, mast_x + 1.5 * s, row_top + 8.0 * s, 10.0 * s, 7.0 * s, flag, w, h);
            }
        }

        // Keep a stable class/carrier icon column for Siege rows.
        let siege_row = board.siege;
        let name_x = if siege_row { player_x + 31.0 * s } else { player_x };
        if siege_row {
            let absolute_icon = entry.siege_icon.and_then(|index| {
                u16::try_from(scoreboard_icon_offset + usize::from(index)).ok()
            });
            player_icon_atlas_cell(
                out,
                shared_icon_count,
                absolute_icon,
                player_x,
                row_top + (row_h - 24.0 * s) * 0.5,
                24.0 * s,
                24.0 * s,
                w,
                h,
            );
        }
        let max_name_chars: usize = if entry.spectating_you {
            if board.show_deaths { 18 } else { 21 }
        } else if siege_row {
            27
        } else if board.team_game {
            30
        } else {
            34
        };
        let max_name_chars =
            max_name_chars.saturating_sub(if ready_column > 0.0 { 6 } else { 0 });
        let name = truncate_jka_text(&entry.name, max_name_chars);
        let row_text = if entry.team == 3 {
            [TEXT[0], TEXT[1], TEXT[2], 0.72]
        } else {
            TEXT
        };
        let ready = (0..16).contains(&entry.client)
            && (board.ready_clients & (1u16 << entry.client as u32)) != 0;
        if ready {
            draw(
                out,
                "READY",
                ready_x,
                row_y,
                header_font_scale,
                fallback_header_scale,
                [0.56, 0.92, 0.66, 1.0],
            );
        }
        draw(
            out,
            &name,
            name_x,
            row_y,
            row_font_scale,
            fallback_row_scale,
            row_text,
        );

        if entry.spectating_you {
            let badge = "SPECTATING YOU";
            let badge_scale = 0.27;
            let badge_fallback_scale = 0.78 * s;
            let badge_text_w = text_width(badge, badge_scale, badge_fallback_scale);
            let badge_w = badge_text_w + 12.0 * s;
            let badge_h = 19.0 * s;
            let badge_right = score_right - 52.0 * s;
            let badge_x = badge_right - badge_w;
            let badge_y = row_top + (row_h - badge_h) * 0.5;
            rect(
                out,
                badge_x,
                badge_y,
                badge_w,
                badge_h,
                [0.12, 0.53, 0.72, 0.17],
                w,
                h,
            );
            rect_outline(
                out,
                badge_x,
                badge_y,
                badge_w,
                badge_h,
                (1.0 * s).max(1.0),
                [0.35, 0.80, 0.96, 0.45],
                w,
                h,
            );
            draw(
                out,
                badge,
                badge_x + 6.0 * s,
                badge_y + badge_h * 0.72,
                badge_scale,
                badge_fallback_scale,
                [0.60, 0.88, 1.0, 0.96],
            );
        }

        if board.ctf {
            // TaystJK prints dashes across the complete CTF stat payload when
            // ping == -1 (connecting/disconnected row).
            let disconnected = entry.ping == -1;
            let values = if disconnected {
                ["-".to_owned(), "-".to_owned(), "-".to_owned(), "-".to_owned(), "-".to_owned(), "-".to_owned()]
            } else {
                [
                    entry.score.to_string(),
                    entry.captures.to_string(),
                    entry.assists.to_string(),
                    entry.defends.to_string(),
                    entry.ping.to_string(),
                    entry.time.to_string(),
                ]
            };
            for (value, right) in [
                (&values[0], score_right),
                (&values[1], captures_right),
                (&values[2], assists_right),
                (&values[3], defends_right),
                (&values[4], ping_right),
                (&values[5], time_right),
            ] {
                let tw = text_width(value, row_font_scale, fallback_row_scale);
                draw(out, value, right - tw, row_y, row_font_scale, fallback_row_scale, if disconnected { TEXT_DIM } else { TEXT });
            }
        } else {
            if !spectating || board.spectator_scores {
                let score = entry.score.to_string();
                let score_w = text_width(&score, row_font_scale, fallback_row_scale);
                draw(out, &score, score_right - score_w, row_y, row_font_scale, fallback_row_scale, TEXT_BRIGHT);
                if board.show_deaths {
                    if let Some(deaths) = entry.deaths {
                        let deaths = deaths.to_string();
                        let deaths_w = text_width(&deaths, row_font_scale, fallback_row_scale);
                        draw(out, &deaths, deaths_right - deaths_w, row_y, row_font_scale, fallback_row_scale, TEXT);
                    }
                }
            }

            let ping = if entry.ping < 0 { "CNCT".to_owned() } else { entry.ping.to_string() };
            let ping_w = text_width(&ping, row_font_scale, fallback_row_scale);
            let ping_color = if entry.ping < 0 {
                TEXT_DIM
            } else if entry.ping <= 60 {
                [0.52, 0.88, 0.66, 1.0]
            } else if entry.ping <= 120 {
                [0.88, 0.84, 0.50, 1.0]
            } else {
                [0.94, 0.55, 0.52, 1.0]
            };
            draw(out, &ping, ping_right - ping_w, row_y, row_font_scale, fallback_row_scale, ping_color);

            let time = entry.time.to_string();
            let time_w = text_width(&time, row_font_scale, fallback_row_scale);
            draw(out, &time, time_right - time_w, row_y, row_font_scale, fallback_row_scale, TEXT);
        }

    }

    if overflow > 0 {
        let footer_top = row_start + used;
        rect(
            out,
            x + 10.0 * s,
            footer_top,
            width - 20.0 * s,
            (1.0 * s).max(1.0),
            [0.42, 0.52, 0.64, 0.20],
            w,
            h,
        );
        let more = format!(
            "+{} MORE PLAYER{}",
            overflow,
            if overflow == 1 { "" } else { "S" }
        );
        draw(
            out,
            &more,
            x + pad,
            footer_top + footer_h * 0.70,
            meta_font_scale,
            fallback_meta_scale,
            TEXT_DIM,
        );
    }
}
