use std::default::Default;
use std::ops::Range;

use x11rb::protocol::{Event, xproto};
use xim::{
    AHashMap, AttributeName, Client, ClientError, ClientHandler, InputStyle, InputStyleList,
    Reader, XimRead,
};

pub enum XimCallbackEvent {
    XimXEvent(x11rb::protocol::Event),
    XimPreeditEvent(xproto::Window, String, Option<Range<usize>>),
    XimCommitEvent(xproto::Window, String),
}

fn fold(ch: char) -> char {
    ch.to_lowercase().next().unwrap_or(ch)
}

fn is_vowel(ch: char) -> bool {
    matches!(
        fold(ch),
        'a' | 'ă' | 'â' | 'e' | 'ê' | 'i' | 'o' | 'ô' | 'ơ' | 'u' | 'ư' | 'y'
            | 'á' | 'à' | 'ả' | 'ã' | 'ạ'
            | 'é' | 'è' | 'ẻ' | 'ẽ' | 'ẹ'
            | 'í' | 'ì' | 'ỉ' | 'ĩ' | 'ị'
            | 'ó' | 'ò' | 'ỏ' | 'õ' | 'ọ'
            | 'ú' | 'ù' | 'ủ' | 'ũ' | 'ụ'
            | 'ý' | 'ỳ' | 'ỷ' | 'ỹ' | 'ỵ'
            | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ'
            | 'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ'
            | 'ế' | 'ề' | 'ể' | 'ễ' | 'ệ'
            | 'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ'
            | 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ'
            | 'ứ' | 'ừ' | 'ử' | 'ữ' | 'ự'
    )
}

fn vowel_slot(ch: char) -> Option<char> {
    match fold(ch) {
        'ư' | 'ứ' | 'ừ' | 'ử' | 'ữ' | 'ự' => Some('ư'),
        'ơ' | 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ' => Some('ơ'),
        'ô' | 'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ' | 'o' | 'ó' | 'ò' | 'ỏ' | 'õ' | 'ọ' => Some('o'),
        'ê' | 'ế' | 'ề' | 'ể' | 'ễ' | 'ệ' | 'e' | 'é' | 'è' | 'ẻ' | 'ẽ' | 'ẹ' => Some('e'),
        'ă' | 'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ'
        | 'a' | 'á' | 'à' | 'ả' | 'ã' | 'ạ' => Some('a'),
        'u' | 'ú' | 'ù' | 'ủ' | 'ũ' | 'ụ' => Some('u'),
        'i' | 'í' | 'ì' | 'ỉ' | 'ĩ' | 'ị' => Some('i'),
        'y' | 'ý' | 'ỳ' | 'ỷ' | 'ỹ' | 'ỵ' => Some('y'),
        _ => None,
    }
}

fn same_slot(a: char, b: char) -> bool {
    a == b || (is_vowel(a) && is_vowel(b))
}

fn is_telex_mark(ch: char) -> bool {
    matches!(fold(ch), 'w' | 's' | 'f' | 'r' | 'x' | 'j' | 'z')
}

fn is_compose_intermediate(prev: char, typed: char) -> bool {
    matches!(
        (vowel_slot(prev), fold(typed)),
        (Some('ư') | Some('u'), 'o') | (Some('i') | Some('y'), 'e')
    )
}

fn is_word_tail(ch: char) -> bool {
    matches!(
        fold(ch),
        'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'c' | 'g' | 'h' | 'm' | 'n' | 'p' | 't'
    ) || matches!(ch, ' ' | ',' | '.' | '?' | '!' | ';' | ':')
}

fn latin_base(ch: char) -> Option<char> {
    match fold(ch) {
        'đ' | 'd' => Some('d'),
        'ư' | 'ứ' | 'ừ' | 'ử' | 'ữ' | 'ự' | 'u' | 'ú' | 'ù' | 'ủ' | 'ũ' | 'ụ' => Some('u'),
        'ơ' | 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ' | 'ô' | 'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ'
        | 'o' | 'ó' | 'ò' | 'ỏ' | 'õ' | 'ọ' => Some('o'),
        'ê' | 'ế' | 'ề' | 'ể' | 'ễ' | 'ệ' | 'e' | 'é' | 'è' | 'ẻ' | 'ẽ' | 'ẹ' => Some('e'),
        'ă' | 'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ'
        | 'a' | 'á' | 'à' | 'ả' | 'ã' | 'ạ' => Some('a'),
        'i' | 'í' | 'ì' | 'ỉ' | 'ĩ' | 'ị' => Some('i'),
        'y' | 'ý' | 'ỳ' | 'ỷ' | 'ỹ' | 'ỵ' => Some('y'),
        _ => None,
    }
}

fn mark_weight(ch: char) -> u8 {
    match fold(ch) {
        'd' | 'a' | 'e' | 'i' | 'o' | 'u' | 'y' => 0,
        'đ' | 'ă' | 'â' | 'ê' | 'ô' | 'ơ' | 'ư' => 1,
        'á' | 'à' | 'ả' | 'ã' | 'ạ' | 'é' | 'è' | 'ẻ' | 'ẽ' | 'ẹ' | 'í' | 'ì' | 'ỉ'
        | 'ĩ' | 'ị' | 'ó' | 'ò' | 'ỏ' | 'õ' | 'ọ' | 'ú' | 'ù' | 'ủ' | 'ũ' | 'ụ' | 'ý'
        | 'ỳ' | 'ỷ' | 'ỹ' | 'ỵ' => 1,
        'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ' | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ' | 'ế' | 'ề' | 'ể'
        | 'ễ' | 'ệ' | 'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ' | 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ' | 'ứ'
        | 'ừ' | 'ử' | 'ữ' | 'ự' => 2,
        _ => 0,
    }
}

fn is_simpler_form(from: char, to: char) -> bool {
    from != to
        && latin_base(from).is_some_and(|base| latin_base(to) == Some(base))
        && mark_weight(to) < mark_weight(from)
}

fn take_backspace_undo(old: &[char], raw: &[char], typed: Option<char>) -> Option<Vec<char>> {
    if typed.is_some() || old.is_empty() || raw.len() != old.len() {
        return None;
    }
    let last = old.len() - 1;
    if old[..last] != raw[..last] || !is_simpler_form(old[last], raw[last]) {
        return None;
    }
    Some(old[..last].to_vec())
}

fn take_ime_length(old: &[char], raw: &[char], caret: i32, typed: Option<char>) -> Option<Vec<char>> {
    if typed.is_some() || old.is_empty() {
        return None;
    }
    let want = usize::try_from(caret).ok()?;
    if want >= old.len() {
        return None;
    }
    let merged = if raw.is_empty() {
        old.to_vec()
    } else {
        merge_preedit(old, raw)
    };
    if merged.len() >= want {
        return Some(merged.into_iter().take(want).collect());
    }
    let mut out = merged;
    out.extend(
        old.iter()
            .copied()
            .skip(out.len())
            .take(want.saturating_sub(out.len())),
    );
    Some(out)
}

fn merge_preedit(old: &[char], raw: &[char]) -> Vec<char> {
    if raw.is_empty() || old.is_empty() {
        return raw.to_vec();
    }
    let mut out = Vec::with_capacity(old.len().max(raw.len()));
    let mut i = 0;
    let mut j = 0;
    while j < raw.len() {
        if i < old.len() && same_slot(old[i], raw[j]) {
            out.push(raw[j]);
            i += 1;
            j += 1;
            if out.last().is_some_and(|ch| is_vowel(*ch)) {
                while i < old.len() && is_word_tail(old[i]) {
                    if j < raw.len() && (raw[j] == old[i] || is_vowel(raw[j])) {
                        break;
                    }
                    out.push(old[i]);
                    i += 1;
                }
            }
            continue;
        }
        out.push(raw[j]);
        if i < old.len() {
            i += 1;
        }
        j += 1;
    }
    out
}

fn append_literal_key(assembled: Vec<char>, caret: i32, typed: Option<char>) -> Vec<char> {
    let want = match usize::try_from(caret) {
        Ok(n) => n,
        Err(_) => return assembled,
    };
    if want != assembled.len() + 1 {
        return assembled;
    }
    let Some(ch) = typed else {
        return assembled;
    };
    if is_telex_mark(ch) || assembled.last() == Some(&ch) || !is_word_tail(ch) {
        return assembled;
    }
    if assembled
        .last()
        .is_some_and(|prev| is_compose_intermediate(*prev, ch))
    {
        return assembled;
    }
    let mut out = assembled;
    out.push(ch);
    out
}

fn caret_says_raw_is_short(caret: i32, raw_len: usize) -> bool {
    match usize::try_from(caret) {
        Ok(want) => want > raw_len,
        Err(_) => true,
    }
}

fn reconstruct_preedit(old: &[char], raw: &[char], caret: i32, typed: Option<char>) -> Vec<char> {
    if let Some(rest) = take_backspace_undo(old, raw, typed) {
        return rest;
    }
    if let Some(rest) = take_ime_length(old, raw, caret, typed) {
        return rest;
    }
    let assembled = if caret_says_raw_is_short(caret, raw.len()) {
        merge_preedit(old, raw)
    } else {
        raw.to_vec()
    };
    append_literal_key(assembled, caret, typed)
}

fn apply_preedit_draw(
    preedit: &mut Vec<char>,
    chg_first: i32,
    chg_len: i32,
    caret: i32,
    status: xim::PreeditDrawStatus,
    replacement: &str,
    typed: Option<char>,
) -> String {
    let old = preedit.clone();
    let raw = if status.contains(xim::PreeditDrawStatus::NO_STRING) {
        Vec::new()
    } else {
        replacement.chars().collect::<Vec<_>>()
    };
    if chg_first == 0 && chg_len > 0 && !raw.is_empty() {
        *preedit = reconstruct_preedit(&old, &raw, caret, typed);
        return preedit.iter().collect();
    }
    let len = preedit.len();
    let start = usize::try_from(chg_first).unwrap_or(0).min(len);
    let delete_len = if chg_len < 0 {
        len.saturating_sub(start)
    } else {
        usize::try_from(chg_len)
            .unwrap_or(0)
            .min(len.saturating_sub(start))
    };
    preedit.splice(start..start + delete_len, raw);
    *preedit = reconstruct_preedit(&old, preedit, caret, typed);
    preedit.iter().collect()
}

fn recover_commit_text(previous: &[char], text: &str) -> String {
    reconstruct_preedit(previous, &text.chars().collect::<Vec<_>>(), -1, None)
        .into_iter()
        .collect()
}

pub(crate) fn xim_debug_log(line: &str) {
    eprintln!("{line}");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/mezon-xim.log")
    {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
}

fn caret_utf16_range(text: &str, caret_chars: i32) -> Option<Range<usize>> {
    if text.is_empty() {
        return None;
    }
    let char_len = text.chars().count();
    let caret = if caret_chars < 0 {
        char_len
    } else {
        usize::try_from(caret_chars).unwrap_or(0).min(char_len)
    };
    let utf16: usize = text.chars().take(caret).map(char::len_utf16).sum();
    Some(utf16..utf16)
}

fn pick_input_style(attributes: &AHashMap<AttributeName, Vec<u8>>) -> InputStyle {
    let styles = attributes
        .get(&AttributeName::QueryInputStyle)
        .and_then(|bytes| {
            let mut reader = Reader::new(bytes);
            InputStyleList::read(&mut reader).ok()
        })
        .map(|list| list.styles)
        .unwrap_or_default();

    // mezon vendor edit: STATUS_CALLBACKS is never implemented by this handler, so
    // prefer every no-status variant over it.
    let preferred = [
        InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING,
        InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NONE,
        InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_CALLBACKS,
        InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NOTHING,
        InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NONE,
        InputStyle::PREEDIT_POSITION | InputStyle::STATUS_AREA,
        InputStyle::PREEDIT_NOTHING | InputStyle::STATUS_NOTHING,
    ];
    for want in preferred {
        if styles.iter().any(|style| *style == want) {
            return want;
        }
    }
    for style in &styles {
        if style.contains(InputStyle::PREEDIT_CALLBACKS) {
            return InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING;
        }
    }
    for style in &styles {
        if style.contains(InputStyle::PREEDIT_POSITION) {
            return InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NOTHING;
        }
    }
    InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING
}

fn usable_locale(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value != "C" && value != "POSIX" && !value.starts_with("C.")
}

fn xim_locale_candidates() -> Vec<String> {
    let mut candidates = Vec::new();
    for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if usable_locale(trimmed) && !candidates.iter().any(|c| c == trimmed) {
                candidates.push(trimmed.to_string());
            }
        }
    }
    for fallback in ["en_US.UTF-8", "C"] {
        if !candidates.iter().any(|c| c == fallback) {
            candidates.push(fallback.to_string());
        }
    }
    candidates
}

pub struct XimHandler {
    pub im_id: u16,
    pub ic_id: u16,
    pub connected: bool,
    pub styles_ready: bool,
    pub opened: bool,
    pub ic_pending: bool,
    pub created_at: std::time::Instant,
    locale_candidates: Vec<String>,
    locale_attempt: usize,
    pub window: xproto::Window,
    pub input_style: InputStyle,
    pub callback_events: Vec<XimCallbackEvent>,
    pub needs_spot_refresh: bool,
    preedit: Vec<char>,
    last_composed: Vec<char>,
    last_applied: Option<String>,
    pub pending_key: Option<char>,
    pub pending_compose_backspace: bool,
}

impl XimHandler {
    pub fn new() -> Self {
        Self {
            im_id: Default::default(),
            ic_id: Default::default(),
            connected: false,
            styles_ready: false,
            opened: false,
            ic_pending: false,
            created_at: std::time::Instant::now(),
            locale_candidates: xim_locale_candidates(),
            locale_attempt: 0,
            window: Default::default(),
            input_style: InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING,
            callback_events: Vec::new(),
            needs_spot_refresh: false,
            preedit: Vec::new(),
            last_composed: Vec::new(),
            last_applied: None,
            pending_key: None,
            pending_compose_backspace: false,
        }
    }

    fn push_callback(&mut self, event: XimCallbackEvent) {
        self.callback_events.push(event);
    }

    pub fn take_callbacks(&mut self) -> Vec<XimCallbackEvent> {
        std::mem::take(&mut self.callback_events)
    }

    fn emit_preedit(&mut self, text: String, caret: i32) {
        self.pending_compose_backspace = false;
        let caret = caret_utf16_range(&text, caret);
        self.push_callback(XimCallbackEvent::XimPreeditEvent(self.window, text, caret));
    }

    fn clear_preedit(&mut self) {
        self.preedit.clear();
    }

    pub fn should_skip_apply(&self, text: &str) -> bool {
        self.last_applied.as_deref() == Some(text)
    }

    pub fn remember_applied(&mut self, text: &str) {
        self.last_applied = Some(text.to_string());
    }

    pub fn take_last_char_backspace(&mut self) -> Option<String> {
        if !self.pending_compose_backspace || self.preedit.len() != 1 {
            self.pending_compose_backspace = false;
            return None;
        }
        self.pending_compose_backspace = false;
        self.preedit.clear();
        self.last_composed.clear();
        Some(String::new())
    }

    fn remember_composed(&mut self) {
        if !self.preedit.is_empty() {
            self.last_composed.clone_from(&self.preedit);
        }
    }

    // mezon vendor edit: an XIM server that rejects the negotiated locale kills the
    // whole client with a protocol error. Instead of losing IME for the session,
    // retry XIM_OPEN with the next fallback locale (en_US.UTF-8, then C).
    pub fn try_reopen_next_locale<C: Client>(&mut self, client: &mut C) -> bool {
        if self.opened {
            return false;
        }
        self.locale_attempt += 1;
        let Some(locale) = self.locale_candidates.get(self.locale_attempt) else {
            return false;
        };
        eprintln!("[xim] open failed; retrying with locale {locale}");
        client.open(locale).is_ok()
    }
}

impl<C: Client<XEvent = xproto::KeyPressEvent>> ClientHandler<C> for XimHandler {
    fn handle_connect(&mut self, client: &mut C) -> Result<(), ClientError> {
        let locale = self
            .locale_candidates
            .get(self.locale_attempt)
            .cloned()
            .unwrap_or_else(|| "en_US.UTF-8".into());
        client.open(&locale)
    }

    fn handle_open(&mut self, client: &mut C, input_method_id: u16) -> Result<(), ClientError> {
        self.im_id = input_method_id;
        self.opened = true;

        client.get_im_values(input_method_id, &[AttributeName::QueryInputStyle])
    }

    fn handle_get_im_values(
        &mut self,
        client: &mut C,
        input_method_id: u16,
        attributes: AHashMap<AttributeName, Vec<u8>>,
    ) -> Result<(), ClientError> {
        self.input_style = pick_input_style(&attributes);
        self.styles_ready = true;
        eprintln!(
            "[xim] negotiated input style callbacks={} position={}",
            self.input_style.contains(InputStyle::PREEDIT_CALLBACKS),
            self.input_style.contains(InputStyle::PREEDIT_POSITION)
        );
        if self.window != 0 && self.ic_id == 0 && !self.ic_pending {
            self.ic_pending = true;
            let ic_attributes = client
                .build_ic_attributes()
                .push(AttributeName::InputStyle, self.input_style)
                .push(AttributeName::ClientWindow, self.window)
                .push(AttributeName::FocusWindow, self.window)
                .build();
            client.create_ic(input_method_id, ic_attributes)?;
        }
        Ok(())
    }

    fn handle_create_ic(
        &mut self,
        client: &mut C,
        input_method_id: u16,
        input_context_id: u16,
    ) -> Result<(), ClientError> {
        self.connected = true;
        self.ic_pending = false;
        self.ic_id = input_context_id;
        self.needs_spot_refresh = true;
        client.set_focus(input_method_id, input_context_id)?;
        Ok(())
    }

    fn handle_commit(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
        text: &str,
    ) -> Result<(), ClientError> {
        let previous = if self.preedit.is_empty() {
            self.last_composed.as_slice()
        } else {
            self.preedit.as_slice()
        };
        let recovered = recover_commit_text(previous, text);
        xim_debug_log(&format!(
            "[xim] commit in={text:?} prev={:?} -> {recovered:?}",
            previous.iter().collect::<String>()
        ));
        self.pending_compose_backspace = false;
        self.clear_preedit();
        self.last_composed.clear();
        self.push_callback(XimCallbackEvent::XimCommitEvent(self.window, recovered));
        Ok(())
    }

    fn handle_forward_event(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
        _flag: xim::ForwardEventFlag,
        xev: C::XEvent,
    ) -> Result<(), ClientError> {
        match xev.response_type {
            x11rb::protocol::xproto::KEY_PRESS_EVENT => {
                xim_debug_log("[xim] forward key press");
                self.push_callback(XimCallbackEvent::XimXEvent(Event::KeyPress(xev)));
            }
            x11rb::protocol::xproto::KEY_RELEASE_EVENT => {
                xim_debug_log("[xim] forward key release");
                self.push_callback(XimCallbackEvent::XimXEvent(Event::KeyRelease(xev)));
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_close(&mut self, client: &mut C, _input_method_id: u16) -> Result<(), ClientError> {
        client.disconnect()
    }

    fn handle_preedit_start(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
    ) -> Result<(), ClientError> {
        Ok(())
    }

    fn handle_preedit_draw(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
        caret: i32,
        chg_first: i32,
        chg_len: i32,
        status: xim::PreeditDrawStatus,
        preedit_string: &str,
        _feedbacks: Vec<xim::Feedback>,
    ) -> Result<(), ClientError> {
        let typed = self.pending_key;
        let text = apply_preedit_draw(
            &mut self.preedit,
            chg_first,
            chg_len,
            caret,
            status,
            preedit_string,
            typed,
        );
        self.remember_composed();
        xim_debug_log(&format!(
            "[xim] preedit draw first={chg_first} len={chg_len} caret={caret} key={typed:?} raw={preedit_string:?} hex={} -> {text:?}",
            preedit_string
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        self.emit_preedit(text, caret);
        Ok(())
    }

    fn handle_preedit_caret(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
        position: &mut i32,
        _direction: xim::CaretDirection,
        _style: xim::CaretStyle,
    ) -> Result<(), ClientError> {
        let text: String = self.preedit.iter().collect();
        self.emit_preedit(text, *position);
        Ok(())
    }

    fn handle_preedit_done(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
    ) -> Result<(), ClientError> {
        xim_debug_log("[xim] preedit done");
        self.remember_composed();
        self.clear_preedit();
        self.emit_preedit(String::new(), 0);
        Ok(())
    }

    fn handle_reset_ic(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
        preedit_text: &str,
    ) -> Result<(), ClientError> {
        let previous = if self.preedit.is_empty() {
            self.last_composed.as_slice()
        } else {
            self.preedit.as_slice()
        };
        let recovered = recover_commit_text(previous, preedit_text);
        self.clear_preedit();
        self.last_composed.clear();
        if recovered.is_empty() {
            self.emit_preedit(String::new(), 0);
        } else {
            self.push_callback(XimCallbackEvent::XimCommitEvent(self.window, recovered));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_preedit_draw, caret_utf16_range, recover_commit_text, reconstruct_preedit,
    };
    use xim::PreeditDrawStatus;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn reconstruct(old: &str, raw: &str, caret: i32, typed: Option<char>) -> String {
        reconstruct_preedit(&chars(old), &chars(raw), caret, typed)
            .into_iter()
            .collect()
    }

    fn draw(preedit: &mut Vec<char>, chg_first: i32, chg_len: i32, replacement: &str) -> String {
        apply_preedit_draw(
            preedit,
            chg_first,
            chg_len,
            -1,
            PreeditDrawStatus::empty(),
            replacement,
            None,
        )
    }

    fn draw_ibus(preedit: &mut Vec<char>, old_len: i32, replacement: &str) -> String {
        let caret = i32::try_from(replacement.chars().count()).unwrap_or(0);
        apply_preedit_draw(
            preedit,
            0,
            old_len,
            caret,
            PreeditDrawStatus::empty(),
            replacement,
            None,
        )
    }

    fn draw_ibus_key(
        preedit: &mut Vec<char>,
        old_len: i32,
        caret: i32,
        replacement: &str,
        typed: char,
    ) -> String {
        apply_preedit_draw(
            preedit,
            0,
            old_len,
            caret,
            PreeditDrawStatus::empty(),
            replacement,
            Some(typed),
        )
    }

    #[test]
    fn preedit_draw_appends_characters() {
        let mut preedit = Vec::new();
        assert_eq!(draw(&mut preedit, 0, 0, "h"), "h");
        assert_eq!(draw(&mut preedit, 1, 0, "o"), "ho");
        assert_eq!(draw(&mut preedit, 2, 0, "a"), "hoa");
    }

    #[test]
    fn preedit_draw_replaces_full_string_like_fcitx5() {
        let mut preedit = Vec::new();
        assert_eq!(draw(&mut preedit, 0, 0, "hoa"), "hoa");
        assert_eq!(draw(&mut preedit, 0, 3, "hóa"), "hóa");
    }

    #[test]
    fn preedit_draw_replaces_a_mid_string_vowel() {
        let mut preedit = Vec::new();
        assert_eq!(draw(&mut preedit, 0, 0, "hoa"), "hoa");
        assert_eq!(draw(&mut preedit, 2, 1, "á"), "hoá");
    }

    #[test]
    fn preedit_draw_no_string_deletes_the_range() {
        let mut preedit = chars("hoa");
        let text = apply_preedit_draw(
            &mut preedit,
            0,
            3,
            -1,
            PreeditDrawStatus::NO_STRING,
            "ignored",
            None,
        );
        assert_eq!(text, "");
        assert!(preedit.is_empty());
    }

    #[test]
    fn preedit_draw_utf8_byte_chg_len_keeps_trailing_consonant() {
        let mut preedit = chars("nguyen");
        assert_eq!(draw(&mut preedit, 4, "ễ".len() as i32, "ễ"), "nguyễn");
        let mut preedit = chars("lai");
        assert_eq!(draw(&mut preedit, 1, "ạ".len() as i32, "ạ"), "lại");
        let mut preedit = chars("đay");
        assert_eq!(draw(&mut preedit, 1, "â".len() as i32, "â"), "đây");
    }

    #[test]
    fn preedit_draw_keeps_coda_when_tone_replaces_vowel_plus_consonant() {
        let mut preedit = chars("nguyen");
        assert_eq!(draw(&mut preedit, 4, 2, "ễ"), "nguyễn");
        let mut preedit = chars("lai");
        assert_eq!(draw(&mut preedit, 1, 2, "ạ"), "lại");
        let mut preedit = chars("đay");
        assert_eq!(draw(&mut preedit, 1, 2, "â"), "đây");
    }

    #[test]
    fn preedit_draw_negative_chg_len_deletes_through_the_end() {
        let mut preedit = chars("hoas");
        assert_eq!(draw(&mut preedit, 2, -1, "á"), "hoá");
    }

    #[test]
    fn preedit_draw_clamps_an_out_of_range_change() {
        let mut preedit = chars("a");
        assert_eq!(draw(&mut preedit, 8, 2, "x"), "ax");
    }

    #[test]
    fn caret_utf16_range_uses_utf16_units() {
        assert_eq!(caret_utf16_range("hóa", 3), Some(3..3));
        assert_eq!(caret_utf16_range("", 0), None);
        assert_eq!(caret_utf16_range("á", -1), Some(1..1));
    }

    #[test]
    fn reconstruct_keeps_old_tail_after_vowel() {
        for (old, raw, want) in [
            ("nguyen", "nguyễ", "nguyễn"),
            ("roi", "rồ", "rồi"),
            ("dau", "đâu", "đâu"),
            ("di", "đị", "đị"),
            ("me", "mẹ", "mẹ"),
            ("may", "mày", "mày"),
            ("nguyen", "nguyễn", "nguyễn"),
            ("nguyễn", "nguyễ", "nguyễn"),
            ("nguyen ", "nguyễ ", "nguyễn "),
            ("di ", "đị", "đị "),
            ("đơn", "đơ", "đơn"),
            ("đơi", "đờ", "đời"),
            ("đươc", "đượ", "được"),
            ("dươn", "dượ", "dượn"),
            ("dượn", "dượ", "dượn"),
            ("dưo", "dươ", "dươ"),
            ("duo", "dươ", "dươ"),
            ("mịa", "mị", "mịa"),
            ("mia", "mị", "mịa"),
        ] {
            assert_eq!(reconstruct(old, raw, -1, None), want, "{old} + {raw}");
            assert_eq!(recover_commit_text(&chars(old), raw), want, "commit {old}");
        }
    }

    #[test]
    fn reconstruct_keeps_backspace_when_caret_matches_raw() {
        for (old, raw, caret, want) in [
            ("nguyễn", "nguyễ", 5, "nguyễ"),
            ("lại", "lạ", 2, "lạ"),
            ("được", "đượ", 3, "đượ"),
            ("đơn", "đơ", 2, "đơ"),
            ("mịa", "mị", 2, "mị"),
            ("đư", "đ", 1, "đ"),
            ("đươ", "đư", 2, "đư"),
            ("dưỡng", "dưỡ", 4, "dưỡn"),
            ("dương", "dưỡ", 4, "dưỡn"),
            ("được", "đượ", 3, "đượ"),
            ("đ", "d", 1, ""),
            ("đư", "đu", 2, "đ"),
            ("đợ", "đơ", 2, "đ"),
        ] {
            assert_eq!(
                reconstruct(old, raw, caret, None),
                want,
                "backspace {old} -> {raw}"
            );
        }
        assert_eq!(reconstruct("đ", "d", 1, Some('d')), "d");
    }

    #[test]
    fn reconstruct_appends_literal_key_when_caret_is_one_longer() {
        for (old, raw, caret, typed, want) in [
            ("đơ", "đơ", 3, 'n', "đơn"),
            ("đơ", "đơ", 3, 'i', "đơi"),
            ("dươ", "dươ", 4, 'n', "dươn"),
            ("dượ", "dượ", 4, 'n', "dượn"),
            ("đươ", "đươ", 4, 'c', "đươc"),
            ("mị", "mị", 3, 'a', "mịa"),
            ("đượ", "đượ", 4, 'j', "đượ"),
            ("dư", "dư", 3, 'o', "dư"),
            ("dươ", "dươ", 3, 'w', "dươ"),
        ] {
            assert_eq!(
                reconstruct(old, raw, caret, Some(typed)),
                want,
                "{old} + {raw} + {typed}"
            );
        }
    }

    #[test]
    fn ibus_full_replace_uses_reconstruct() {
        let mut preedit = Vec::new();
        assert_eq!(draw_ibus(&mut preedit, 0, "nguyen"), "nguyen");
        assert_eq!(draw_ibus_key(&mut preedit, 6, 6, "nguyễ", 'x'), "nguyễn");
        assert_eq!(draw_ibus(&mut preedit, 5, "nguyễn"), "nguyễn");

        let mut preedit = chars("nguyen");
        let text = apply_preedit_draw(
            &mut preedit,
            0,
            6,
            8,
            PreeditDrawStatus::empty(),
            "nguyễn",
            None,
        );
        assert_eq!(text, "nguyễn");

        let mut preedit = Vec::new();
        assert_eq!(draw_ibus(&mut preedit, 0, "đư"), "đư");
        assert_eq!(draw_ibus(&mut preedit, 2, "đươ"), "đươ");
        assert_eq!(draw_ibus_key(&mut preedit, 3, 4, "đươ", 'c'), "đươc");
        assert_eq!(draw_ibus_key(&mut preedit, 4, 4, "đượ", 'j'), "được");
        assert_eq!(recover_commit_text(&preedit, "đượ"), "được");
    }
}
