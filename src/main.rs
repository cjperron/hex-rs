#![allow(clippy::too_many_arguments)]
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, MouseEvent, MouseEventKind, EnableMouseCapture, DisableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use hex_rs::{Agent, AppConfig, AppState, HexCell, ClickAgent, RandomAgent, GameUpdate, HexBoard, HexMove, Player};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Modifier},
    text::Span,
    widgets::{
        canvas::{Canvas, Context, Line as CanvasLine},
        Block, Borders, Paragraph, List, ListItem, ListState, Clear,
    },
    Frame, Terminal,
};
use std::{
    io,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

// ── State ────────────────────────────────────────────────────────────────────

struct MenuState {
    selected: usize,
    dropdown_open: bool,
    dropdown_list: ListState,
}

impl MenuState {
    fn new() -> Self {
        Self { selected: 0, dropdown_open: false, dropdown_list: ListState::default() }
    }

    fn open_dropdown(&mut self, config: &AppConfig) {
        self.dropdown_open = true;
        self.dropdown_list.select(Some(config_to_dropdown_idx(config, self.selected)));
    }

    fn close_dropdown(&mut self) {
        self.dropdown_open = false;
    }
}

#[derive(Default)]
struct CanvasInfo {
    rect: Rect,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    reset_btn: Rect,
    menu_btn: Rect,
    history_rect: Rect,
}

// ── Actions ──────────────────────────────────────────────────────────────────

enum MenuAction { StartGame, Quit }

enum GameAction {
    Quit,
    GoToMenu,
    Reset,
    MoveCursor(usize, usize),
    PlaceMove(usize, usize),
    ScrollHistory(i32),
}

// ── Small helpers ────────────────────────────────────────────────────────────

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

type AgentFactory = fn(std::sync::Arc<std::sync::Mutex<Receiver<HexMove>>>) -> Box<dyn Agent>;

fn create_human_agent(rx: std::sync::Arc<std::sync::Mutex<Receiver<HexMove>>>) -> Box<dyn Agent> {
    Box::new(ClickAgent::new(rx))
}

fn create_random_agent(_rx: std::sync::Arc<std::sync::Mutex<Receiver<HexMove>>>) -> Box<dyn Agent> {
    Box::new(RandomAgent::new())
}

const AVAILABLE_AGENTS: &[(&str, AgentFactory)] = &[
    ("Human", create_human_agent),
    ("Random", create_random_agent),
];

fn dropdown_max(selected: usize) -> usize {
    match selected { 0 | 1 => 27, 4 | 5 => AVAILABLE_AGENTS.len().saturating_sub(1), _ => 1 }
}

fn dropdown_options(selected: usize) -> Vec<ListItem<'static>> {
    match selected {
        0 | 1 => (3usize..=30).map(|v| ListItem::new(format!("  {}  ", v))).collect(),
        2     => vec![ListItem::new("  Enabled  "), ListItem::new("  Disabled  ")],
        3     => vec![ListItem::new("  Red  "), ListItem::new("  Blue  ")],
        4 | 5 => AVAILABLE_AGENTS.iter().map(|(name, _)| ListItem::new(format!("  {}  ", name))).collect(),
        _     => vec![],
    }
}

fn config_to_dropdown_idx(config: &AppConfig, selected: usize) -> usize {
    match selected {
        0 => config.width.saturating_sub(3),
        1 => config.height.saturating_sub(3),
        2 => if config.swap_rule { 0 } else { 1 },
        3 => if config.first_player == Player::Red { 0 } else { 1 },
        4 => AVAILABLE_AGENTS.iter().position(|&(n, _)| n == config.agent1).unwrap_or(0),
        5 => AVAILABLE_AGENTS.iter().position(|&(n, _)| n == config.agent2).unwrap_or(0),
        _ => 0,
    }
}

fn apply_dropdown_selection(config: &mut AppConfig, selected: usize, idx: usize) {
    match selected {
        0 => config.width = idx + 3,
        1 => config.height = idx + 3,
        2 => config.swap_rule = idx == 0,
        3 => config.first_player = if idx == 0 { Player::Red } else { Player::Blue },
        4 => if let Some(&(name, _)) = AVAILABLE_AGENTS.get(idx) { config.agent1 = name.to_string(); },
        5 => if let Some(&(name, _)) = AVAILABLE_AGENTS.get(idx) { config.agent2 = name.to_string(); },
        _ => {}
    }
    config.save();
}

fn menu_rects(size: Rect) -> (Rect, Rect) {
    let (w, h) = (40, 20);
    let outer = Rect::new(
        size.width.saturating_sub(w) / 2,
        size.height.saturating_sub(h) / 2,
        w.min(size.width),
        h.min(size.height),
    );
    let inner = Rect::new(outer.x + 2, outer.y + 2, outer.width.saturating_sub(4), outer.height.saturating_sub(4));
    (outer, inner)
}

// ── Game thread ──────────────────────────────────────────────────────────────

fn spawn_game_thread(board: HexBoard, mut agent1: Box<dyn Agent>, mut agent2: Box<dyn Agent>, update_tx: Sender<GameUpdate>) {
    thread::spawn(move || {
        let mut b = board;
        let _ = update_tx.send(GameUpdate::State(b.clone()));
        loop {
            if b.winner.is_some() { break; }
            let action = if b.current_player == Player::Red {
                agent1.get_move(&b)
            } else {
                agent2.get_move(&b)
            };
            match b.apply_move(&action) {
                Ok(_)  => { if update_tx.send(GameUpdate::State(b.clone())).is_err() { break; } }
                Err(e) => { if update_tx.send(GameUpdate::Error(e)).is_err() { break; } }
            }
        }
    });
}

fn create_agent(name: &str, agent_rx: std::sync::Arc<std::sync::Mutex<Receiver<HexMove>>>) -> Box<dyn Agent> {
    for &(agent_name, factory) in AVAILABLE_AGENTS {
        if agent_name == name {
            return factory(agent_rx);
        }
    }
    // Default fallback
    AVAILABLE_AGENTS[0].1(agent_rx)
}

fn launch_game(config: &AppConfig) -> (HexBoard, Sender<HexMove>, Receiver<GameUpdate>) {
    let (ui_tx, agent_rx) = mpsc::channel::<HexMove>();
    let (game_tx, ui_rx) = mpsc::channel::<GameUpdate>();
    let board = HexBoard::new(config.width, config.height, config.first_player, config.swap_rule);
    let rx_arc = std::sync::Arc::new(std::sync::Mutex::new(agent_rx));
    
    let agent1 = create_agent(&config.agent1, rx_arc.clone());
    let agent2 = create_agent(&config.agent2, rx_arc);
    
    spawn_game_thread(board.clone(), agent1, agent2, game_tx);
    (board, ui_tx, ui_rx)
}

// ── Menu drawing ─────────────────────────────────────────────────────────────

fn build_menu_list_items(config: &AppConfig, menu: &MenuState) -> Vec<ListItem<'static>> {
    let items = [
        format!("{:15} [ {} ▼ ]",   "Board Width:",  config.width),
        format!("{:15} [ {} ▼ ]",   "Board Height:", config.height),
        format!("{:15} [ {} ▼ ]",   "Swap Rule:",    if config.swap_rule { "Enabled" } else { "Disabled" }),
        format!("{:15} [ {:?} ▼ ]", "First Player:", config.first_player),
        format!("{:15} [ {} ▼ ]",   "Red Agent:",    config.agent1),
        format!("{:15} [ {} ▼ ]",   "Blue Agent:",   config.agent2),
        String::new(),
        "  [ Reset to Defaults ]".to_string(),
        "  [ Start Game ]".to_string(),
    ];

    items.into_iter().enumerate().map(|(i, text)| {
        if i == 6 { return ListItem::new(""); }
        let state_idx = if i > 6 { i - 1 } else { i };
        let style = match (state_idx == menu.selected, menu.dropdown_open) {
            (true, false) => Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD),
            (true, true)  => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            _             => Style::default(),
        };
        ListItem::new(text).style(style)
    }).collect()
}

fn draw_menu(f: &mut Frame, config: &AppConfig, menu: &mut MenuState) {
    let (outer, inner) = menu_rects(f.area());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Hex Engine Setup ");

    f.render_widget(Clear, outer);
    f.render_widget(block, outer);
    f.render_widget(List::new(build_menu_list_items(config, menu)), inner);

    if !menu.dropdown_open { return; }

    let dropdown_rect = Rect::new(inner.x + 16, inner.y + menu.selected as u16 + 1, 14, 6);
    let drop_list = List::new(dropdown_options(menu.selected))
        .block(Block::default().borders(Borders::ALL).style(Style::default().bg(Color::Black)))
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));

    f.render_widget(Clear, dropdown_rect);
    f.render_stateful_widget(drop_list, dropdown_rect, &mut menu.dropdown_list);
}

// ── Board drawing ────────────────────────────────────────────────────────────

fn draw_board_borders(ctx: &mut Context<'_>, board: &HexBoard, hex_r: f64, hex_w: f64) {
    let sqrt3 = 3.0f64.sqrt();
    let y_margin = hex_r * 1.25;
    let x_margin = hex_w * 0.6;

    for i in 0..4 {
        let out  = i as f64 * 0.5;
        let y_top = y_margin + out;
        let y_bot = -(board.height as f64 - 1.0) * 1.5 * hex_r - y_margin - out;
        let xm   = x_margin + out;
        let cols = (board.width as f64 - 1.0) * hex_w;

        let (x_tl, x_tr) = (-y_top / sqrt3 - xm, -y_top / sqrt3 + cols + xm);
        let (x_bl, x_br) = (-y_bot / sqrt3 - xm, -y_bot / sqrt3 + cols + xm);

        ctx.draw(&CanvasLine { x1: x_tl, y1: y_top, x2: x_tr, y2: y_top, color: Color::Red  });
        ctx.draw(&CanvasLine { x1: x_bl, y1: y_bot, x2: x_br, y2: y_bot, color: Color::Red  });
        ctx.draw(&CanvasLine { x1: x_tl, y1: y_top, x2: x_bl, y2: y_bot, color: Color::Blue });
        ctx.draw(&CanvasLine { x1: x_tr, y1: y_top, x2: x_br, y2: y_bot, color: Color::Blue });
    }
}

fn hex_vertices(cx: f64, cy: f64, r: f64) -> [(f64, f64); 6] {
    std::array::from_fn(|i| {
        let angle = (60.0 * i as f64 - 30.0_f64).to_radians();
        (cx + r * angle.cos(), cy + r * angle.sin())
    })
}

fn draw_hex_ring(ctx: &mut Context<'_>, cx: f64, cy: f64, r: f64, color: Color) {
    let pts = hex_vertices(cx, cy, r);
    for i in 0..6 {
        let (p1, p2) = (pts[i], pts[(i + 1) % 6]);
        ctx.draw(&CanvasLine { x1: p1.0, y1: p1.1, x2: p2.0, y2: p2.1, color });
    }
}

fn draw_hex_filled(ctx: &mut Context<'_>, cx: f64, cy: f64, hex_r: f64, color: Color) {
    let mut r = hex_r - 0.5;
    while r > 0.0 {
        draw_hex_ring(ctx, cx, cy, r, color);
        r -= 0.5;
    }
}

fn paint_board(
    ctx: &mut Context<'_>,
    board: &HexBoard,
    cursor_x: usize,
    cursor_y: usize,
    current_color: Color,
    hex_r: f64,
    hex_w: f64,
) {
    draw_board_borders(ctx, board, hex_r, hex_w);

    for y in 0..board.height {
        for x in 0..board.width {
            let cx = x as f64 * hex_w + y as f64 * (hex_w / 2.0);
            let cy = -(y as f64 * 1.5 * hex_r);
            let is_hovered = x == cursor_x && y == cursor_y;

            let (mut color, filled) = match board.grid[y][x] {
                HexCell::Empty                  => (Color::DarkGray, false),
                HexCell::Occupied(Player::Red)  => (Color::Red,      true),
                HexCell::Occupied(Player::Blue) => (Color::Blue,     true),
            };
            if is_hovered && !filled { color = current_color; }

            draw_hex_ring(ctx, cx, cy, hex_r, color);
            if filled {
                draw_hex_filled(ctx, cx, cy, hex_r, color);
            } else if is_hovered {
                ctx.print(cx, cy, Span::styled("·", Style::default().fg(color)));
            }
        }
    }
}

fn draw_game(
    f: &mut Frame,
    board: &HexBoard,
    config: &AppConfig,
    cursor_x: usize,
    cursor_y: usize,
    error_msg: &str,
    hex_r: f64,
    hex_w: f64,
    canvas_info: &mut CanvasInfo,
    history_state: &mut ratatui::widgets::ListState,
) {
    let size = f.area();
    let border_color  = board.winner.map(|p| p.color()).unwrap_or_else(|| board.current_player.color());
    let current_color = board.current_player.color();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Fill(1), Constraint::Length(8)])
        .split(size);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Length(20), Constraint::Length(25)])
        .split(chunks[0]);

    let status = match board.winner {
        Some(w) => format!("Winner: {:?}", w),
        None    => format!("Current turn: {:?}", board.current_player),
    };

    f.render_widget(
        Paragraph::new(status)
            .style(Style::default().fg(border_color).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(border_color)).title("Hex Engine")),
        top_chunks[0],
    );

    canvas_info.reset_btn = top_chunks[1];
    canvas_info.menu_btn  = top_chunks[2];

    f.render_widget(
        Paragraph::new(" [r] Reset Game ")
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow))),
        canvas_info.reset_btn,
    );
    f.render_widget(
        Paragraph::new(" [m] Main Menu ")
            .style(Style::default().fg(Color::Cyan))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan))),
        canvas_info.menu_btn,
    );

    let board_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled("Board (Canvas mode: click cells or use arrow keys)", Style::default().fg(Color::Yellow)));

    canvas_info.rect = board_block.inner(chunks[1]);

    let total_w = board.width as f64 * hex_w + board.height as f64 * (hex_w / 2.0);
    let total_h = board.height as f64 * (1.5 * hex_r) + 0.5 * hex_r;
    let padding = hex_r * 3.0;
    canvas_info.x_bounds = [-padding, total_w + padding];
    canvas_info.y_bounds = [-(total_h + padding), padding];

    let canvas = Canvas::default()
        .block(board_block)
        .x_bounds(canvas_info.x_bounds)
        .y_bounds(canvas_info.y_bounds)
        .marker(ratatui::symbols::Marker::Braille)
        .paint(|ctx| paint_board(ctx, board, cursor_x, cursor_y, current_color, hex_r, hex_w));

    f.render_widget(canvas, chunks[1]);

    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    f.render_widget(
        Paragraph::new(error_msg.to_string())
            .style(Style::default().fg(Color::LightRed))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(border_color)).title("Logs")),
        bottom_chunks[0],
    );

    let move_items: Vec<ListItem> = board.move_history
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let player = if i % 2 == 0 { config.first_player } else { config.first_player.opposite() };
            ListItem::new(ratatui::text::Line::from(vec![
                Span::raw(format!("{}. ", i + 1)),
                Span::styled(format!("{}", m), Style::default().fg(player.color())),
            ]))
        })
        .collect();

    canvas_info.history_rect = bottom_chunks[1];

    f.render_stateful_widget(
        List::new(move_items)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(border_color)).title("Move History")),
        bottom_chunks[1],
        history_state,
    );
}

// ── Menu event handlers ──────────────────────────────────────────────────────

fn handle_dropdown_key(key: KeyEvent, config: &mut AppConfig, menu: &mut MenuState) {
    match key.code {
        KeyCode::Esc => menu.close_dropdown(),
        KeyCode::Up => {
            if let Some(i) = menu.dropdown_list.selected() {
                menu.dropdown_list.select(Some(i.saturating_sub(1)));
            }
        }
        KeyCode::Down => {
            if let Some(i) = menu.dropdown_list.selected() {
                let max = dropdown_max(menu.selected);
                if i < max { menu.dropdown_list.select(Some(i + 1)); }
            }
        }
        KeyCode::Enter => {
            if let Some(i) = menu.dropdown_list.selected() {
                apply_dropdown_selection(config, menu.selected, i);
            }
            menu.close_dropdown();
        }
        _ => {}
    }
}

fn handle_menu_enter(config: &mut AppConfig, menu: &mut MenuState) -> Option<MenuAction> {
    match menu.selected {
        0..=5 => menu.open_dropdown(config),
        6 => { *config = AppConfig::default(); config.save(); }
        7 => return Some(MenuAction::StartGame),
        _ => {}
    }
    None
}

fn handle_menu_key(key: KeyEvent, config: &mut AppConfig, menu: &mut MenuState) -> Option<MenuAction> {
    if menu.dropdown_open {
        handle_dropdown_key(key, config, menu);
        return None;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(MenuAction::Quit),
        KeyCode::Down  => { menu.selected = if menu.selected >= 7 { 0 } else { menu.selected + 1 }; None }
        KeyCode::Up    => { menu.selected = if menu.selected == 0 { 7 } else { menu.selected - 1 }; None }
        KeyCode::Enter => handle_menu_enter(config, menu),
        _ => None,
    }
}

fn handle_dropdown_click(mx: u16, my: u16, config: &mut AppConfig, menu: &mut MenuState, inner: Rect) {
    let dropdown_rect = Rect::new(inner.x + 16, inner.y + menu.selected as u16 + 1, 14, 6);

    if !rect_contains(dropdown_rect, mx, my) {
        menu.close_dropdown();
        return;
    }

    let inner_y   = dropdown_rect.y + 1;
    let inner_bot = dropdown_rect.y + dropdown_rect.height - 1;
    if my < inner_y || my >= inner_bot { return; }

    let clicked_idx = (my - inner_y) as usize + menu.dropdown_list.offset();
    let max_items   = match menu.selected { 0 | 1 => 28, 4 | 5 => AVAILABLE_AGENTS.len(), _ => 2 };

    if clicked_idx < max_items {
        menu.dropdown_list.select(Some(clicked_idx));
        apply_dropdown_selection(config, menu.selected, clicked_idx);
        menu.close_dropdown();
    }
}

fn handle_menu_click(mx: u16, my: u16, config: &mut AppConfig, menu: &mut MenuState, inner: Rect) -> Option<MenuAction> {
    if menu.dropdown_open {
        handle_dropdown_click(mx, my, config, menu, inner);
        return None;
    }
    if !rect_contains(inner, mx, my) { return None; }

    let visual_to_state: [Option<usize>; 9] = [Some(0), Some(1), Some(2), Some(3), Some(4), Some(5), None, Some(6), Some(7)];
    let Some(Some(target)) = visual_to_state.get((my - inner.y) as usize) else { return None; };
    menu.selected = *target;
    handle_menu_enter(config, menu)
}

fn handle_menu_scroll(kind: MouseEventKind, menu: &mut MenuState) {
    if menu.dropdown_open {
        let Some(i) = menu.dropdown_list.selected() else { return; };
        let max = dropdown_max(menu.selected);
        match kind {
            MouseEventKind::ScrollUp                  => menu.dropdown_list.select(Some(i.saturating_sub(1))),
            MouseEventKind::ScrollDown if i < max     => menu.dropdown_list.select(Some(i + 1)),
            _ => {}
        }
        return;
    }
    menu.selected = match kind {
        MouseEventKind::ScrollUp => if menu.selected == 0 { 5 } else { menu.selected - 1 },
        _                        => if menu.selected >= 5 { 0 } else { menu.selected + 1 },
    };
}

fn handle_menu_mouse(mouse: MouseEvent, config: &mut AppConfig, menu: &mut MenuState, size: Rect) -> Option<MenuAction> {
    let (_, inner) = menu_rects(size);
    match mouse.kind {
        MouseEventKind::Down(event::MouseButton::Left) => {
            handle_menu_click(mouse.column, mouse.row, config, menu, inner)
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            handle_menu_scroll(mouse.kind, menu);
            None
        }
        _ => None,
    }
}

// ── Game event handlers ──────────────────────────────────────────────────────

fn find_closest_hex(canvas_x: f64, canvas_y: f64, board: &HexBoard, hex_r: f64, hex_w: f64) -> Option<(usize, usize)> {
    let mut best_dist = f64::MAX;
    let mut best = None;

    for y in 0..board.height {
        for x in 0..board.width {
            let cx = x as f64 * hex_w + y as f64 * (hex_w / 2.0);
            let cy = -(y as f64 * 1.5 * hex_r);
            let dist = (canvas_x - cx).powi(2) + (canvas_y - cy).powi(2);
            if dist < best_dist {
                best_dist = dist;
                best = Some((x, y));
            }
        }
    }

    best.filter(|_| best_dist <= hex_r * hex_r)
}

fn screen_to_canvas(mx: u16, my: u16, info: &CanvasInfo) -> (f64, f64) {
    let rel_x = (mx - info.rect.x) as f64 / info.rect.width as f64;
    let rel_y = 1.0 - (my - info.rect.y) as f64 / info.rect.height as f64;
    (
        info.x_bounds[0] + rel_x * (info.x_bounds[1] - info.x_bounds[0]),
        info.y_bounds[0] + rel_y * (info.y_bounds[1] - info.y_bounds[0]),
    )
}

fn handle_game_key(key: KeyEvent, board: &HexBoard, cursor_x: usize, cursor_y: usize) -> Option<GameAction> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc                => Some(GameAction::Quit),
        KeyCode::Char('m')                               => Some(GameAction::GoToMenu),
        KeyCode::Char('r')                               => Some(GameAction::Reset),
        KeyCode::Up    if cursor_y > 0                   => Some(GameAction::MoveCursor(cursor_x, cursor_y - 1)),
        KeyCode::Down  if cursor_y < board.height - 1    => Some(GameAction::MoveCursor(cursor_x, cursor_y + 1)),
        KeyCode::Left  if cursor_x > 0                   => Some(GameAction::MoveCursor(cursor_x - 1, cursor_y)),
        KeyCode::Right if cursor_x < board.width - 1     => Some(GameAction::MoveCursor(cursor_x + 1, cursor_y)),
        KeyCode::Enter                                   => Some(GameAction::PlaceMove(cursor_x, cursor_y)),
        _                                                => None,
    }
}

fn handle_game_mouse(mouse: MouseEvent, info: &CanvasInfo, board: &HexBoard, hex_r: f64, hex_w: f64) -> Option<GameAction> {
    let (mx, my) = (mouse.column, mouse.row);
    let is_click = mouse.kind == MouseEventKind::Down(event::MouseButton::Left);

    if rect_contains(info.history_rect, mx, my) {
        match mouse.kind {
            MouseEventKind::ScrollDown => return Some(GameAction::ScrollHistory(1)),
            MouseEventKind::ScrollUp   => return Some(GameAction::ScrollHistory(-1)),
            _ => ()
        }
    }

    if is_click {
        if rect_contains(info.reset_btn, mx, my) { return Some(GameAction::Reset); }
        if rect_contains(info.menu_btn,  mx, my) { return Some(GameAction::GoToMenu); }
    }

    if info.rect.width == 0 || !rect_contains(info.rect, mx, my) { return None; }

    let (canvas_x, canvas_y) = screen_to_canvas(mx, my, info);
    let (hx, hy) = find_closest_hex(canvas_x, canvas_y, board, hex_r, hex_w)?;

    if is_click { Some(GameAction::PlaceMove(hx, hy)) } else { Some(GameAction::MoveCursor(hx, hy)) }
}

// ── Event polling ────────────────────────────────────────────────────────────

fn poll_menu_event(
    config: &mut AppConfig,
    menu: &mut MenuState,
    size: Rect,
) -> Result<Option<MenuAction>, Box<dyn std::error::Error>> {
    if !event::poll(Duration::from_millis(50))? { return Ok(None); }
    let action = match event::read()? {
        Event::Key(k)   => handle_menu_key(k, config, menu),
        Event::Mouse(m) => handle_menu_mouse(m, config, menu, size),
        _               => None,
    };
    Ok(action)
}

fn poll_game_event(
    info: &CanvasInfo,
    board: &HexBoard,
    cursor_x: usize,
    cursor_y: usize,
    hex_r: f64,
    hex_w: f64,
) -> Result<Option<GameAction>, Box<dyn std::error::Error>> {
    if !event::poll(Duration::from_millis(16))? { return Ok(None); }
    let action = match event::read()? {
        Event::Key(k)   => handle_game_key(k, board, cursor_x, cursor_y),
        Event::Mouse(m) => handle_game_mouse(m, info, board, hex_r, hex_w),
        _               => None,
    };
    Ok(action)
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let mut app_state   = AppState::Menu;
    let mut config      = AppConfig::load();
    let mut menu        = MenuState::new();
    let mut canvas_info = CanvasInfo::default();

    let hex_r = 10.0_f64;
    let hex_w = hex_r * 3.0_f64.sqrt();

    let mut ui_tx: Option<Sender<HexMove>>      = None;
    let mut ui_rx: Option<Receiver<GameUpdate>> = None;
    let mut current_board = HexBoard::new(1, 1, Player::Red, false);
    let mut cursor_x  = 0_usize;
    let mut cursor_y  = 0_usize;
    let mut error_msg = String::new();
    let mut history_state = ratatui::widgets::ListState::default();

    loop {
        match app_state {
            AppState::Menu => {
                terminal.draw(|f| draw_menu(f, &config, &mut menu))?;

                let Some(action) = poll_menu_event(&mut config, &mut menu, terminal.size()?.into())? else { continue };
                match action {
                    MenuAction::Quit => break,
                    MenuAction::StartGame => {
                        let (board, tx, rx) = launch_game(&config);
                        current_board = board;
                        ui_tx     = Some(tx);
                        ui_rx     = Some(rx);
                        cursor_x  = config.width / 2;
                        cursor_y  = config.height / 2;
                        error_msg = String::new();
                        app_state = AppState::Playing;
                    }
                }
            }

            AppState::Playing => {
                if let Some(rx) = &ui_rx {
                    while let Ok(update) = rx.try_recv() {
                        match update {
                            GameUpdate::State(b) => {
                                current_board = b;
                                error_msg = String::new();
                            },
                            GameUpdate::Error(e) => error_msg = e,
                        }
                    }
                }

                terminal.draw(|f| {
                    draw_game(f, &current_board, &config, cursor_x, cursor_y, &error_msg, hex_r, hex_w, &mut canvas_info, &mut history_state);
                })?;

                let Some(action) = poll_game_event(&canvas_info, &current_board, cursor_x, cursor_y, hex_r, hex_w)? else { continue };
                match action {
                    GameAction::Quit     => break,
                    GameAction::GoToMenu => { app_state = AppState::Menu; ui_tx = None; ui_rx = None; }
                    GameAction::Reset    => {
                        let (board, tx, rx) = launch_game(&config);
                        current_board = board;
                        ui_tx     = Some(tx);
                        ui_rx     = Some(rx);
                        cursor_x  = config.width / 2;
                        cursor_y  = config.height / 2;
                        error_msg = String::new();
                    }
                    GameAction::MoveCursor(x, y) => { cursor_x = x; cursor_y = y; }
                    GameAction::ScrollHistory(delta) => {
                        let total = current_board.move_history.len();
                        if total > 0 {
                            let current = history_state.selected().unwrap_or(total.saturating_sub(1));
                            let next = (current as i32 + delta).clamp(0, total.saturating_sub(1) as i32) as usize;
                            history_state.select(Some(next));
                        }
                    }
                    GameAction::PlaceMove(x, y)  => {
                        cursor_x = x;
                        cursor_y = y;
                        let candidate = HexMove { x, y };
                        if let Some(tx) = &ui_tx {
                            let _ = tx.send(candidate);
                        }
                    }
                }
            }
        }
    }

    teardown(terminal)
}

fn teardown(mut terminal: Terminal<CrosstermBackend<io::Stdout>>) -> Result<(), Box<dyn std::error::Error>> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}
