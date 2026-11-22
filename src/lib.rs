//! Gri Wordle: a full-screen Wordle app for Grimoire 2.3+.
//!
//! The app fetches the day's answer from the public NYT Wordle endpoint, keeps the six guesses
//! and their marks in the app's own `state/runtime.bin`, draws the whole game with the Grimoire
//! app ABI, and shows Grimoire's shared status bar above the game. It is no_std: fixed buffers
//! only, no allocator, no filesystem and no background threads.

#![no_std]

mod words;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    // The host has a fuel limit, so a bug cannot spin forever.
    loop {}
}

#[link(wasm_import_module = "grimoire")]
extern "C" {
    fn fill_rect(x: i32, y: i32, w: i32, h: i32, gray: i32);
    fn outline_rect(x: i32, y: i32, w: i32, h: i32, thickness: i32, gray: i32);
    fn text(
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        ptr: *const u8,
        len: i32,
        px_milli: i32,
        face: i32,
        center: i32,
        gray: i32,
    );
    fn state_get(ptr: *mut u8, cap: i32) -> i32;
    fn state_set(ptr: *const u8, len: i32);
    fn status(ptr: *const u8, len: i32);
    fn request_redraw(after_ms: i32);
    #[link_name = "toast"]
    fn host_toast(ptr: *const u8, len: i32, ms: i32);
    fn date_text(ptr: *mut u8, cap: i32) -> i32;
    fn http_start(
        method_ptr: *const u8,
        method_len: i32,
        url_ptr: *const u8,
        url_len: i32,
        headers_ptr: *const u8,
        headers_len: i32,
        body_ptr: *const u8,
        body_len: i32,
    ) -> i32;
    fn http_poll() -> i32;
    fn http_status() -> i32;
    fn http_body(ptr: *mut u8, cap: i32) -> i32;
    fn http_cancel();
}

const API: &[u8] = b"https://www.nytimes.com/svc/wordle/v2/";
const STATE_MAGIC: &[u8; 4] = b"WRD1";
const STATE_LEN: usize = 83;
const MAX_BODY: usize = 4096;

const MARK_NONE: u8 = 0;
const MARK_ABSENT: u8 = 1;
const MARK_PRESENT: u8 = 2;
const MARK_CORRECT: u8 = 3;

const NET_NEEDS_FETCH: u8 = 0;
const NET_FETCHING: u8 = 1;
const NET_READY: u8 = 2;
const NET_ERROR: u8 = 3;

const RESULT_PLAYING: u8 = 0;
const RESULT_WON: u8 = 1;
const RESULT_LOST: u8 = 2;

const KEY_ENTER: u8 = 26;
const KEY_DELETE: u8 = 27;

const ROW0: [u8; 10] = [16, 22, 4, 17, 19, 24, 20, 8, 14, 15]; // QWERTYUIOP
const ROW1: [u8; 9] = [0, 18, 3, 5, 6, 7, 9, 10, 11]; // ASDFGHJKL
const ROW2: [u8; 7] = [25, 23, 2, 21, 1, 13, 12]; // ZXCVBNM

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

const EMPTY_RECT: Rect = Rect::new(0, 0, 0, 0);

#[derive(Clone, Copy)]
struct Key {
    rect: Rect,
    code: u8,
}

const EMPTY_KEY: Key = Key {
    rect: EMPTY_RECT,
    code: 0,
};

#[derive(Clone, Copy)]
struct Layout {
    keys: [Key; 28],
    key_count: u8,
    title: Rect,
    date: Rect,
    header_h: i32,
    board_x: i32,
    board_y: i32,
    tile: i32,
    gap: i32,
    message: Rect,
}

const EMPTY_LAYOUT: Layout = Layout {
    keys: [EMPTY_KEY; 28],
    key_count: 0,
    title: EMPTY_RECT,
    date: EMPTY_RECT,
    header_h: 0,
    board_x: 0,
    board_y: 0,
    tile: 0,
    gap: 0,
    message: EMPTY_RECT,
};

struct Game {
    initialized: bool,
    date: [u8; 10],
    date_len: u8,
    answer: [u8; 5],
    guesses: [[u8; 5]; 6],
    marks: [[u8; 5]; 6],
    row: u8,
    cursor: u8,
    result: u8,
    net: u8,
    http_body: [u8; MAX_BODY],
    layout: Layout,
}

const NEW_GAME: Game = Game {
    initialized: false,
    date: [0; 10],
    date_len: 0,
    answer: [0; 5],
    guesses: [[0; 5]; 6],
    marks: [[0; 5]; 6],
    row: 0,
    cursor: 0,
    result: RESULT_PLAYING,
    net: NET_NEEDS_FETCH,
    http_body: [0; MAX_BODY],
    layout: EMPTY_LAYOUT,
};

static mut GAME: Game = NEW_GAME;

fn game() -> &'static mut Game {
    // The wasm module is single-threaded. Every entry point takes the one game for its call.
    unsafe { &mut *core::ptr::addr_of_mut!(GAME) }
}

// ---- tiny drawing wrappers ----

fn draw_fill(rect: Rect, gray: u8) {
    if rect.w > 0 && rect.h > 0 {
        unsafe { fill_rect(rect.x, rect.y, rect.w, rect.h, gray as i32) }
    }
}

fn draw_outline(rect: Rect, thickness: i32, gray: u8) {
    if rect.w > 0 && rect.h > 0 {
        unsafe { outline_rect(rect.x, rect.y, rect.w, rect.h, thickness, gray as i32) }
    }
}

fn draw_text(rect: Rect, bytes: &[u8], px: i32, face: i32, center: bool, gray: u8) {
    if rect.w <= 0 || rect.h <= 0 || bytes.is_empty() {
        return;
    }
    unsafe {
        text(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            bytes.as_ptr(),
            bytes.len() as i32,
            px.max(1) * 1000,
            face,
            i32::from(center),
            gray as i32,
        )
    }
}

fn draw_static(rect: Rect, value: &'static str, px: i32, face: i32, center: bool, gray: u8) {
    draw_text(rect, value.as_bytes(), px, face, center, gray);
}

fn toast(value: &'static str) {
    unsafe { host_toast(value.as_ptr(), value.len() as i32, 2400) }
}

fn request_redraw_ms(ms: i32) {
    unsafe { request_redraw(ms) }
}

// ---- date, state and the daily answer ----

fn today() -> ([u8; 10], u8) {
    let mut date = [0u8; 10];
    let n = unsafe { date_text(date.as_mut_ptr(), date.len() as i32) };
    if n == date.len() as i32 {
        (date, n as u8)
    } else {
        ([0u8; 10], 0)
    }
}

fn cancel_http(g: &mut Game) {
    unsafe { http_cancel() };
    if g.net == NET_FETCHING {
        g.net = NET_NEEDS_FETCH;
    }
}

fn reset_play(g: &mut Game) {
    g.answer = [0; 5];
    g.guesses = [[0; 5]; 6];
    g.marks = [[0; 5]; 6];
    g.row = 0;
    g.cursor = 0;
    g.result = RESULT_PLAYING;
    g.net = NET_NEEDS_FETCH;
}

fn valid_answer(answer: &[u8; 5]) -> bool {
    answer.iter().all(|b| b.is_ascii_uppercase())
}

fn load_state(g: &mut Game) -> bool {
    let mut buf = [0u8; 128];
    let n = unsafe { state_get(buf.as_mut_ptr(), buf.len() as i32) };
    if n < STATE_LEN as i32 || &buf[0..4] != STATE_MAGIC {
        return false;
    }
    let date_len = buf[4] as usize;
    if date_len != 10 || buf[5..15] != g.date {
        return false;
    }
    let mut answer = [0u8; 5];
    answer.copy_from_slice(&buf[15..20]);
    if !valid_answer(&answer) {
        return false;
    }
    let row = buf[20];
    let cursor = buf[21];
    let result = buf[22];
    if row > 6 || cursor > 5 || result > RESULT_LOST {
        return false;
    }
    let mut guesses = [[0u8; 5]; 6];
    let mut marks = [[0u8; 5]; 6];
    for r in 0..6 {
        for c in 0..5 {
            let letter = buf[23 + r * 5 + c];
            let mark = buf[53 + r * 5 + c];
            if (letter != 0 && !letter.is_ascii_uppercase()) || mark > MARK_CORRECT {
                return false;
            }
            guesses[r][c] = letter;
            marks[r][c] = mark;
        }
    }
    for r in 0..row as usize {
        if guesses[r].contains(&0) {
            return false;
        }
    }
    for c in 0..cursor as usize {
        if guesses[row as usize][c] == 0 {
            return false;
        }
    }
    g.answer = answer;
    g.guesses = guesses;
    g.marks = marks;
    g.row = row;
    g.cursor = cursor;
    g.result = result;
    g.net = NET_READY;
    true
}

fn save_state(g: &Game) {
    if !valid_answer(&g.answer) || g.date_len != 10 {
        return;
    }
    let mut buf = [0u8; STATE_LEN];
    buf[0..4].copy_from_slice(STATE_MAGIC);
    buf[4] = 10;
    buf[5..15].copy_from_slice(&g.date);
    buf[15..20].copy_from_slice(&g.answer);
    buf[20] = g.row;
    buf[21] = g.cursor;
    buf[22] = g.result;
    for r in 0..6 {
        for c in 0..5 {
            buf[23 + r * 5 + c] = g.guesses[r][c];
            buf[53 + r * 5 + c] = g.marks[r][c];
        }
    }
    unsafe { state_set(buf.as_ptr(), buf.len() as i32) };
}

fn start_fetch(g: &mut Game) {
    cancel_http(g);
    if g.date_len != 10 {
        g.net = NET_ERROR;
        return;
    }
    let mut url = [0u8; 80];
    let mut n = 0usize;
    for &b in API {
        url[n] = b;
        n += 1;
    }
    for &b in &g.date[..10] {
        url[n] = b;
        n += 1;
    }
    for &b in b".json" {
        url[n] = b;
        n += 1;
    }
    let method = b"GET";
    let ret = unsafe {
        http_start(
            method.as_ptr(),
            method.len() as i32,
            url.as_ptr(),
            n as i32,
            core::ptr::null(),
            0,
            core::ptr::null(),
            0,
        )
    };
    if ret == 0 {
        g.net = NET_FETCHING;
        request_redraw_ms(100);
    } else {
        g.net = NET_ERROR;
        toast("Could not start the daily download.");
    }
}

fn parse_solution(body: &[u8]) -> Option<[u8; 5]> {
    let needle = b"\"solution\"";
    let mut i = 0usize;
    while i + needle.len() <= body.len() {
        if &body[i..i + needle.len()] != needle {
            i += 1;
            continue;
        }
        let mut j = i + needle.len();
        while j < body.len() && body[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= body.len() || body[j] != b':' {
            i += 1;
            continue;
        }
        j += 1;
        while j < body.len() && body[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= body.len() || body[j] != b'"' || j + 6 > body.len() {
            return None;
        }
        j += 1;
        let mut answer = [0u8; 5];
        for (k, slot) in answer.iter_mut().enumerate() {
            let b = body[j + k];
            if !b.is_ascii_alphabetic() {
                return None;
            }
            *slot = b.to_ascii_uppercase();
        }
        if body[j + 5] != b'"' {
            return None;
        }
        return Some(answer);
    }
    None
}

fn fail_fetch(g: &mut Game, message: &'static str) {
    cancel_http(g);
    g.net = NET_ERROR;
    toast(message);
}

fn poll_fetch(g: &mut Game) {
    if g.net != NET_FETCHING {
        return;
    }
    match unsafe { http_poll() } {
        0 => request_redraw_ms(100),
        1 => {
            let code = unsafe { http_status() };
            let n = unsafe { http_body(g.http_body.as_mut_ptr(), MAX_BODY as i32) };
            unsafe { http_cancel() };
            if code != 200 {
                g.net = NET_ERROR;
                toast("The daily word could not be downloaded.");
                return;
            }
            let n = n.clamp(0, MAX_BODY as i32) as usize;
            if let Some(answer) = parse_solution(&g.http_body[..n]) {
                g.answer = answer;
                g.net = NET_READY;
                g.result = RESULT_PLAYING;
                save_state(g);
            } else {
                g.net = NET_ERROR;
                toast("The daily word was not in the answer.");
            }
        }
        _ => fail_fetch(g, "The daily word could not be downloaded."),
    }
}

fn prepare(g: &mut Game) {
    let (date, date_len) = today();
    if !g.initialized {
        g.initialized = true;
        if date_len == 10 {
            g.date = date;
            g.date_len = date_len;
            if !load_state(g) {
                reset_play(g);
            }
        } else {
            g.net = NET_ERROR;
        }
    } else if date_len == 10 && (g.date_len != 10 || g.date != date) {
        cancel_http(g);
        g.date = date;
        g.date_len = date_len;
        reset_play(g);
    }
    if g.net == NET_NEEDS_FETCH {
        start_fetch(g);
    }
    if g.net == NET_FETCHING {
        poll_fetch(g);
        if g.net == NET_FETCHING {
            request_redraw_ms(100);
        }
    }
}

// ---- the game ----

fn enter_letter(g: &mut Game, letter: u8) {
    if g.net != NET_READY || g.result != RESULT_PLAYING || g.row >= 6 || g.cursor >= 5 {
        return;
    }
    g.guesses[g.row as usize][g.cursor as usize] = letter;
    g.cursor += 1;
    save_state(g);
}

fn backspace(g: &mut Game) {
    if g.net != NET_READY || g.result != RESULT_PLAYING || g.row >= 6 || g.cursor == 0 {
        return;
    }
    g.cursor -= 1;
    g.guesses[g.row as usize][g.cursor as usize] = 0;
    save_state(g);
}

fn valid_word(guess: &[u8; 5], answer: &[u8; 5]) -> bool {
    if guess == answer {
        return true;
    }
    let mut low = 0usize;
    let mut high = words::WORD_COUNT;
    while low < high {
        let mid = low + (high - low) / 2;
        let start = mid * words::WORD_BYTES;
        let word = &words::WORDS[start..start + words::WORD_BYTES];
        match word.cmp(&guess[..]) {
            core::cmp::Ordering::Less => low = mid + 1,
            core::cmp::Ordering::Greater => high = mid,
            core::cmp::Ordering::Equal => return true,
        }
    }
    false
}

fn evaluate(guess: &[u8; 5], answer: &[u8; 5]) -> [u8; 5] {
    let mut marks = [MARK_ABSENT; 5];
    let mut used = [false; 5];
    for i in 0..5 {
        if guess[i] == answer[i] {
            marks[i] = MARK_CORRECT;
            used[i] = true;
        }
    }
    for i in 0..5 {
        if marks[i] == MARK_CORRECT {
            continue;
        }
        for j in 0..5 {
            if !used[j] && guess[i] == answer[j] {
                marks[i] = MARK_PRESENT;
                used[j] = true;
                break;
            }
        }
    }
    marks
}

fn submit(g: &mut Game) {
    if g.net != NET_READY || g.result != RESULT_PLAYING || g.row >= 6 {
        return;
    }
    if g.cursor != 5 {
        toast("Not enough letters.");
        return;
    }
    let row = g.row as usize;
    let guess = g.guesses[row];
    if !valid_word(&guess, &g.answer) {
        toast("Not in the word list.");
        return;
    }
    g.marks[row] = evaluate(&guess, &g.answer);
    g.row += 1;
    g.cursor = 0;
    if guess == g.answer {
        g.result = RESULT_WON;
    } else if g.row == 6 {
        g.result = RESULT_LOST;
    }
    save_state(g);
}

fn on_key(g: &mut Game, code: u8) {
    if code < 26 {
        enter_letter(g, b'A' + code);
    } else if code == KEY_ENTER {
        submit(g);
    } else if code == KEY_DELETE {
        backspace(g);
    }
}

// ---- layout and UI ----

fn add_key(layout: &mut Layout, rect: Rect, code: u8) {
    let at = layout.key_count as usize;
    if at < layout.keys.len() {
        layout.keys[at] = Key { rect, code };
        layout.key_count += 1;
    }
}

fn compute_layout(w: i32, h: i32) -> Layout {
    let mut layout = EMPTY_LAYOUT;
    let margin = (w / 35).max(18);
    let top = (h / 55).max(14);
    let title_px = (w / 12).min(h / 28).max(36);
    let date_px = (title_px * 36 / 100).max(22);
    layout.title = Rect::new(margin, top, w - margin * 2, title_px + 10);
    layout.date = Rect::new(margin, top + title_px + 10, w - margin * 2, date_px + 6);
    layout.header_h = top + title_px + 10 + date_px + 6 + 14;

    let gap = (w / 130).max(6);
    layout.gap = gap;
    let bottom = (h / 60).max(16);
    let keyboard_h = (h * 24 / 100).min(620);
    let key_gap = (w / 230).max(6);
    let key_h = (keyboard_h - key_gap * 2) / 3;
    let keyboard_y = h - bottom - keyboard_h;
    let keyboard_margin = (w / 35).max(18);
    let usable = w - keyboard_margin * 2;
    let unit = (usable - key_gap * 9) / 10;

    let row0_y = keyboard_y;
    for (i, &code) in ROW0.iter().enumerate() {
        let x = keyboard_margin + i as i32 * (unit + key_gap);
        add_key(&mut layout, Rect::new(x, row0_y, unit, key_h), code);
    }
    let row1_y = row0_y + key_h + key_gap;
    let row1_width = unit * 9 + key_gap * 8;
    let row1_x = (w - row1_width) / 2;
    for (i, &code) in ROW1.iter().enumerate() {
        let x = row1_x + i as i32 * (unit + key_gap);
        add_key(&mut layout, Rect::new(x, row1_y, unit, key_h), code);
    }
    let row2_y = row1_y + key_h + key_gap;
    add_key(
        &mut layout,
        Rect::new(keyboard_margin, row2_y, unit, key_h),
        KEY_ENTER,
    );
    for (i, &code) in ROW2.iter().enumerate() {
        let x = keyboard_margin + (i as i32 + 1) * (unit + key_gap);
        add_key(&mut layout, Rect::new(x, row2_y, unit, key_h), code);
    }
    let delete_x = keyboard_margin + 8 * (unit + key_gap);
    add_key(
        &mut layout,
        Rect::new(delete_x, row2_y, unit, key_h),
        KEY_DELETE,
    );

    let message_h = (h / 48).max(34);
    let keyboard_top = keyboard_y;
    let board_top = layout.header_h + 14;
    let space_below_board = keyboard_top - 22 - message_h;
    let tile_by_height = (space_below_board - board_top - gap * 5) / 6;
    let tile_by_width = (w - margin * 2 - gap * 4) / 5;
    let tile = tile_by_height.min(tile_by_width).max(36);
    let board_w = tile * 5 + gap * 4;
    let board_h = tile * 6 + gap * 5;
    layout.board_x = (w - board_w) / 2;
    layout.board_y = board_top + ((space_below_board - board_top - board_h) / 2).max(0);
    layout.tile = tile;
    layout.message = Rect::new(
        margin,
        layout.board_y + board_h + 14,
        w - margin * 2,
        message_h,
    );
    layout
}

fn letter_state(g: &Game, letter: u8) -> u8 {
    let mut best = MARK_NONE;
    for r in 0..g.row as usize {
        for c in 0..5 {
            if g.guesses[r][c] == letter && g.marks[r][c] > best {
                best = g.marks[r][c];
            }
        }
    }
    best
}

fn draw_header(g: &Game, w: i32) {
    draw_fill(Rect::new(0, 0, w, g.layout.header_h), 0);
    draw_static(
        g.layout.title,
        "WORDLE",
        (g.layout.title.h - 10).max(24),
        1,
        true,
        255,
    );
    if g.date_len == 10 {
        draw_text(
            g.layout.date,
            &g.date,
            (g.layout.date.h - 6).max(18),
            4,
            true,
            235,
        );
    } else {
        draw_static(
            g.layout.date,
            "TODAY",
            (g.layout.date.h - 6).max(18),
            4,
            true,
            235,
        );
    }
}

fn draw_board(g: &Game) {
    let tile = g.layout.tile;
    let gap = g.layout.gap;
    for r in 0..6usize {
        for c in 0..5usize {
            let x = g.layout.board_x + c as i32 * (tile + gap);
            let y = g.layout.board_y + r as i32 * (tile + gap);
            let rect = Rect::new(x, y, tile, tile);
            let letter = g.guesses[r][c];
            let mark = if r < g.row as usize {
                g.marks[r][c]
            } else {
                MARK_NONE
            };
            let fill = match mark {
                MARK_CORRECT => 0,
                MARK_PRESENT => 132,
                MARK_ABSENT => 214,
                _ => 255,
            };
            draw_fill(rect, fill);
            let thickness = if g.result == RESULT_PLAYING && r == g.row as usize {
                5
            } else {
                2
            };
            draw_outline(rect, thickness, 0);
            if letter != 0 {
                let ink = if fill < 128 { 255 } else { 0 };
                let glyph = [letter];
                draw_text(rect, &glyph, tile * 46 / 100, 1, true, ink);
            }
        }
    }
}

fn draw_message(g: &mut Game) {
    let rect = g.layout.message;
    let px = (rect.h * 72 / 100).max(28);
    if g.net == NET_ERROR {
        draw_fill(rect, 0);
        draw_static(rect, "TAP TO RETRY", px, 1, true, 255);
        return;
    }
    if g.net == NET_NEEDS_FETCH || g.net == NET_FETCHING {
        draw_static(rect, "FETCHING TODAY'S WORD...", px, 3, true, 0);
        return;
    }
    if g.result == RESULT_WON {
        let mut line = [0u8; 32];
        let mut n = 0usize;
        for &b in b"YOU WON IN " {
            line[n] = b;
            n += 1;
        }
        line[n] = b'0' + g.row.min(9);
        n += 1;
        line[n] = b'/';
        n += 1;
        line[n] = b'6';
        n += 1;
        draw_text(rect, &line[..n], px, 1, true, 0);
        return;
    }
    if g.result == RESULT_LOST {
        let mut line = [0u8; 32];
        let mut n = 0usize;
        for &b in b"THE WORD WAS " {
            line[n] = b;
            n += 1;
        }
        for &b in &g.answer {
            line[n] = b;
            n += 1;
        }
        draw_text(rect, &line[..n], px, 1, true, 0);
        return;
    }
    if g.cursor == 5 {
        draw_static(rect, "PRESS ENTER", px, 3, true, 0);
    } else {
        let mut line = [0u8; 32];
        let mut n = 0usize;
        for &b in b"GUESS " {
            line[n] = b;
            n += 1;
        }
        line[n] = b'1' + g.row.min(8);
        n += 1;
        for &b in b" OF 6" {
            line[n] = b;
            n += 1;
        }
        draw_text(rect, &line[..n], px, 3, true, 0);
    }
}

fn draw_keyboard(g: &Game) {
    let ready = g.net == NET_READY;
    let active = ready && g.result == RESULT_PLAYING;
    for key in &g.layout.keys[..g.layout.key_count as usize] {
        let (fill, ink) = if !ready {
            (238, 110)
        } else if key.code < 26 {
            match letter_state(g, b'A' + key.code) {
                MARK_CORRECT => (0, 255),
                MARK_PRESENT => (132, 255),
                MARK_ABSENT => (214, 0),
                _ if active => (252, 0),
                _ => (238, 110),
            }
        } else if active {
            (240, 0)
        } else {
            (238, 110)
        };
        draw_fill(key.rect, fill);
        draw_outline(key.rect, 2, 0);
        if key.code < 26 {
            let glyph = [b'A' + key.code];
            draw_text(
                key.rect,
                &glyph,
                (key.rect.h * 38 / 100).max(14),
                1,
                true,
                ink,
            );
        } else if key.code == KEY_ENTER {
            draw_static(
                key.rect,
                "ENTER",
                (key.rect.h * 22 / 100).max(10),
                1,
                true,
                ink,
            );
        } else {
            draw_static(
                key.rect,
                "DEL",
                (key.rect.h * 26 / 100).max(11),
                1,
                true,
                ink,
            );
        }
    }
}

fn publish_status(g: &Game) {
    let mut line = [0u8; 96];
    let mut n = 0usize;
    let push = |line: &mut [u8; 96], n: &mut usize, bytes: &[u8]| {
        for &b in bytes {
            if *n < line.len() {
                line[*n] = b;
                *n += 1;
            }
        }
    };
    match g.net {
        NET_NEEDS_FETCH => push(&mut line, &mut n, b"starting"),
        NET_FETCHING => push(&mut line, &mut n, b"fetching"),
        NET_ERROR => push(&mut line, &mut n, b"error"),
        _ if g.result == RESULT_WON => {
            push(&mut line, &mut n, b"won:");
            push(&mut line, &mut n, &g.answer);
            push(&mut line, &mut n, b":");
            line[n] = b'0' + g.row;
            n += 1;
        }
        _ if g.result == RESULT_LOST => {
            push(&mut line, &mut n, b"lost:");
            push(&mut line, &mut n, &g.answer);
        }
        _ => {
            push(&mut line, &mut n, b"playing:");
            for c in 0..g.cursor as usize {
                let letter = g.guesses[g.row as usize][c];
                if letter != 0 {
                    line[n] = letter;
                    n += 1;
                }
            }
        }
    }
    unsafe { status(line.as_ptr(), n as i32) };
}

fn draw(g: &mut Game, w: i32, h: i32) {
    g.layout = compute_layout(w, h);
    draw_fill(Rect::new(0, 0, w, h), 255);
    draw_header(g, w);
    draw_board(g);
    draw_message(g);
    draw_keyboard(g);
    publish_status(g);
}

fn on_pointer(g: &mut Game, phase: i32, x: i32, y: i32) {
    if phase != 2 {
        return;
    }
    if g.net == NET_ERROR {
        if g.layout.message.contains(x, y) {
            start_fetch(g);
        }
        return;
    }
    if g.net != NET_READY || g.result != RESULT_PLAYING {
        return;
    }
    for key in &g.layout.keys[..g.layout.key_count as usize] {
        if key.rect.contains(x, y) {
            on_key(g, key.code);
            return;
        }
    }
}

// ---- the app ABI ----

#[no_mangle]
pub extern "C" fn app_draw(width: i32, height: i32, _scale: i32, _elapsed: i32) {
    let g = game();
    prepare(g);
    draw(g, width, height);
}

#[no_mangle]
pub extern "C" fn app_pointer(phase: i32, x: i32, y: i32, _source: i32) {
    on_pointer(game(), phase, x, y);
}

#[no_mangle]
pub extern "C" fn app_status_bar() -> i32 {
    1
}

#[no_mangle]
pub extern "C" fn app_stop() {
    save_state(game());
}
