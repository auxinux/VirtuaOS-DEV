//! The wizard: state machine, key handling, and ratatui rendering.

use crate::install::{read_progress, spawn_backend, write_conf, Progress};
use crate::model::{
    cidr_ok, detect_disks, detect_ifaces, dns_list_ok, hostname_ok, ip_ok, password_ok, scan_wifi,
    Answers, Choice, Disk, NetIface, WifiNet, KEYMAPS, LOCALES, NET_METHODS, TIMEZONES, UI_LANGS,
};
use crate::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use std::process::Child;

/// One screen of the wizard. The first 14 variants form the linear [`FLOW`];
/// `Installing`/`Done`/`Failed` are entered only from `Confirm` once the
/// backend has been launched and are not part of that back/forward walk.
#[derive(PartialEq, Clone, Copy)]
pub enum Step {
    UiLang,
    Locale,
    Keyboard,
    Hostname,
    Timezone,
    NetIface,
    WifiScan,
    WifiPass,
    NetMethod,
    NetStatic,
    RootPwd,
    AdminPwd,
    Disk,
    Confirm,
    /// Backend is running; screen is driven by [`App::poll_install`].
    Installing,
    Done,
    Failed,
}

/// Full linear walk-through. Steps that don't apply to the current answers
/// (WiFi steps on a wired NIC, the static-IP form under DHCP, the whole network
/// section when no NIC is present) are skipped by `applies()`.
const FLOW: [Step; 14] = [
    Step::UiLang,
    Step::Locale,
    Step::Keyboard,
    Step::Hostname,
    Step::Timezone,
    Step::NetIface,
    Step::WifiScan,
    Step::WifiPass,
    Step::NetMethod,
    Step::NetStatic,
    Step::RootPwd,
    Step::AdminPwd,
    Step::Disk,
    Step::Confirm,
];

/// Which half of the enter/confirm password dance a `RootPwd`/`AdminPwd`
/// step is currently in.
#[derive(PartialEq, Clone, Copy)]
enum PwPhase {
    Enter,
    Confirm,
}

/// What the event loop should do after a key.
pub enum Action {
    None,
    Quit,
    Reboot,
}

/// The whole wizard's mutable state: current step, collected answers, and
/// the scratch fields each step's key handler reads/writes while the user
/// is still typing (before the value is committed into `ans`).
pub struct App {
    pub step: Step,
    ans: Answers,
    /// Selected index for the current step's menu/list (`UiLang`, `Disk`, …).
    sel: usize,
    /// Free-text input buffer for the current step (hostname, password, one
    /// field of the static-IP form, …).
    input: String,
    /// Second buffer, used only for password confirmation.
    input2: String,
    pw_phase: PwPhase,
    /// Validation error shown under the current step's input, if any.
    err: String,
    disks: Vec<Disk>,
    ifaces: Vec<NetIface>,
    wifis: Vec<WifiNet>,
    /// Whether the WiFi network picked in `WifiScan` requires a password
    /// (decides whether `WifiPass` is reached — see [`App::applies`]).
    wifi_secured: bool,
    /// Which sub-field of the multi-field `NetStatic` form is active (0 =
    /// address, 1 = gateway, 2 = DNS).
    field_idx: usize,
    child: Option<Child>,
    progress: Option<Progress>,
    fail_msg: String,
    /// Incremented every event-loop iteration while installing, to drive the
    /// spinner animation.
    tick: u64,
}

impl App {
    pub fn new() -> Self {
        let mut app = App {
            step: Step::UiLang,
            ans: Answers::default(),
            sel: 0,
            input: String::new(),
            input2: String::new(),
            pw_phase: PwPhase::Enter,
            err: String::new(),
            disks: detect_disks(),
            ifaces: detect_ifaces(),
            wifis: Vec::new(),
            wifi_secured: false,
            field_idx: 0,
            child: None,
            progress: None,
            fail_msg: String::new(),
            tick: 0,
        };
        app.enter_step(Step::UiLang);
        app
    }

    fn fr(&self) -> bool {
        self.ans.ui_lang == "fr"
    }
    fn t(&self, en: &'static str, fr: &'static str) -> &'static str {
        if self.fr() {
            fr
        } else {
            en
        }
    }

    /// Static option list backing a menu step (empty for non-menu steps).
    fn menu(&self) -> &'static [Choice] {
        match self.step {
            Step::UiLang => UI_LANGS,
            Step::Locale => LOCALES,
            Step::Keyboard => KEYMAPS,
            Step::Timezone => TIMEZONES,
            Step::NetMethod => NET_METHODS,
            _ => &[],
        }
    }

    fn menu_len(&self) -> usize {
        match self.step {
            Step::Disk => self.disks.len(),
            Step::NetIface => self.ifaces.len(),
            Step::WifiScan => self.wifis.len(),
            _ => self.menu().len(),
        }
    }

    /// Does this step apply given the answers so far? Used to skip the WiFi
    /// steps on a wired NIC, the static-IP form under DHCP, and the whole
    /// network section when there is no configurable interface.
    fn applies(&self, step: Step) -> bool {
        match step {
            Step::NetIface => !self.ifaces.is_empty(),
            Step::WifiScan => self.ans.net_kind == "wifi",
            Step::WifiPass => self.ans.net_kind == "wifi" && self.wifi_secured,
            Step::NetMethod => !self.ifaces.is_empty(),
            Step::NetStatic => !self.ifaces.is_empty() && self.ans.net_method == "static",
            _ => true,
        }
    }

    fn enter_step(&mut self, step: Step) {
        self.step = step;
        self.err.clear();
        self.input.clear();
        self.input2.clear();
        self.pw_phase = PwPhase::Enter;
        self.field_idx = 0;
        self.sel = 0;
        match step {
            Step::Hostname => {
                self.input = if self.ans.hostname.is_empty() {
                    "virtuaos".to_string()
                } else {
                    self.ans.hostname.clone()
                };
            }
            Step::UiLang => self.sel = self.find(UI_LANGS, &self.ans.ui_lang),
            Step::Locale => self.sel = self.find(LOCALES, &self.ans.lang_code),
            Step::Keyboard => self.sel = self.find(KEYMAPS, &self.ans.keymap),
            Step::Timezone => self.sel = self.find(TIMEZONES, &self.ans.tz),
            Step::NetIface => {
                self.sel = self
                    .ifaces
                    .iter()
                    .position(|i| i.dev == self.ans.net_iface)
                    .unwrap_or(0);
            }
            Step::WifiScan => {
                // Scan synchronously when entering; nmcli rescan takes ~1-3 s.
                let dev = self.ans.net_iface.clone();
                self.wifis = scan_wifi(&dev);
                self.sel = self
                    .wifis
                    .iter()
                    .position(|w| w.ssid == self.ans.wifi_ssid)
                    .unwrap_or(0);
            }
            Step::WifiPass => self.input = self.ans.wifi_password.clone(),
            Step::NetMethod => self.sel = self.find(NET_METHODS, &self.ans.net_method),
            Step::NetStatic => {
                self.field_idx = 0;
                self.input = self.ans.net_address.clone();
            }
            _ => {}
        }
    }

    fn find(&self, list: &[Choice], value: &str) -> usize {
        list.iter().position(|c| c.value == value).unwrap_or(0)
    }

    fn next(&mut self) {
        if let Some(i) = FLOW.iter().position(|s| *s == self.step) {
            for s in &FLOW[i + 1..] {
                if self.applies(*s) {
                    self.enter_step(*s);
                    return;
                }
            }
        }
    }

    fn prev(&mut self) -> Action {
        if let Some(i) = FLOW.iter().position(|s| *s == self.step) {
            for s in FLOW[..i].iter().rev() {
                if self.applies(*s) {
                    self.enter_step(*s);
                    return Action::None;
                }
            }
            return Action::Quit; // nothing applicable before this → exit wizard
        }
        Action::None
    }

    // ── key handling ──────────────────────────────────────────────────────────

    /// Dispatch a key press to the handler for the current step. Ctrl-C
    /// always quits, regardless of step.
    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        match self.step {
            Step::UiLang
            | Step::Locale
            | Step::Keyboard
            | Step::Timezone
            | Step::Disk
            | Step::NetIface
            | Step::WifiScan
            | Step::NetMethod => self.key_menu(key),
            Step::WifiPass => self.key_wifi_pass(key),
            Step::NetStatic => self.key_static(key),
            Step::Hostname => self.key_hostname(key),
            Step::RootPwd | Step::AdminPwd => self.key_password(key),
            Step::Confirm => self.key_confirm(key),
            Step::Installing => Action::None,
            Step::Done => match key.code {
                KeyCode::Char('r') | KeyCode::Char('R') => Action::Reboot,
                _ => Action::Quit,
            },
            Step::Failed => Action::Quit,
        }
    }

    /// Handle a key press on any list/menu step (up/down to move `sel`,
    /// Enter to commit the selection into `ans` and advance, Esc to go back).
    fn key_menu(&mut self, key: KeyEvent) -> Action {
        let len = self.menu_len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if len > 0 {
                    self.sel = (self.sel + len - 1) % len;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.sel = (self.sel + 1) % len;
                }
            }
            // Re-scan WiFi on demand.
            KeyCode::Char('r') | KeyCode::Char('R') if self.step == Step::WifiScan => {
                let dev = self.ans.net_iface.clone();
                self.wifis = scan_wifi(&dev);
                self.sel = 0;
            }
            KeyCode::Enter => {
                if len == 0 {
                    return Action::None;
                }
                match self.step {
                    Step::UiLang => {
                        self.ans.ui_lang = UI_LANGS[self.sel].value.to_string();
                        // Seed sensible locale/keymap defaults from UI language.
                        if self.ans.lang_code.is_empty() {
                            self.ans.lang_code =
                                if self.fr() { "fr_CA.UTF-8" } else { "en_US.UTF-8" }.to_string();
                        }
                    }
                    Step::Locale => self.ans.lang_code = LOCALES[self.sel].value.to_string(),
                    Step::Keyboard => self.ans.keymap = KEYMAPS[self.sel].value.to_string(),
                    Step::Timezone => self.ans.tz = TIMEZONES[self.sel].value.to_string(),
                    Step::Disk => self.ans.disk = self.disks[self.sel].dev.clone(),
                    Step::NetIface => {
                        let ifc = self.ifaces[self.sel].clone();
                        self.ans.net_iface = ifc.dev.clone();
                        self.ans.net_kind = ifc.kind_str().to_string();
                        if !ifc.is_wifi() {
                            self.ans.wifi_ssid.clear();
                            self.ans.wifi_password.clear();
                            self.wifi_secured = false;
                        }
                    }
                    Step::WifiScan => {
                        let w = self.wifis[self.sel].clone();
                        self.ans.wifi_ssid = w.ssid.clone();
                        self.wifi_secured = w.secured;
                        if !w.secured {
                            self.ans.wifi_password.clear();
                        }
                    }
                    Step::NetMethod => {
                        self.ans.net_method = NET_METHODS[self.sel].value.to_string()
                    }
                    _ => {}
                }
                self.next();
            }
            KeyCode::Esc => return self.prev(),
            _ => {}
        }
        Action::None
    }

    /// Free-text entry for the WiFi passphrase step.
    fn key_wifi_pass(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Enter => {
                // WPA PSK must be 8-63 chars; only enforced when the network is
                // secured (we never reach this step for an open network).
                if self.input.chars().count() >= 8 {
                    self.ans.wifi_password = self.input.clone();
                    self.next();
                } else {
                    self.err = self
                        .t(
                            "WiFi password must be at least 8 characters.",
                            "Le mot de passe WiFi doit comporter au moins 8 caractères.",
                        )
                        .to_string();
                }
            }
            KeyCode::Esc => return self.prev(),
            _ => {}
        }
        Action::None
    }

    // Static-IP form: 3 sub-fields (address/CIDR, gateway, DNS) entered in turn.
    fn key_static(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Enter => {
                let v = self.input.trim().to_string();
                match self.field_idx {
                    0 => {
                        if !cidr_ok(&v) {
                            self.err = self
                                .t(
                                    "Invalid address. Use IP/prefix, e.g. 192.168.1.50/24",
                                    "Adresse invalide. Format IP/préfixe, ex. 192.168.1.50/24",
                                )
                                .to_string();
                            return Action::None;
                        }
                        self.ans.net_address = v;
                        self.field_idx = 1;
                        self.input = self.ans.net_gateway.clone();
                        self.err.clear();
                    }
                    1 => {
                        if !v.is_empty() && !ip_ok(&v) {
                            self.err = self
                                .t(
                                    "Invalid gateway IP (or leave empty).",
                                    "Passerelle invalide (ou laissez vide).",
                                )
                                .to_string();
                            return Action::None;
                        }
                        self.ans.net_gateway = v;
                        self.field_idx = 2;
                        self.input = if self.ans.net_dns.is_empty() {
                            "1.1.1.1 8.8.8.8".to_string()
                        } else {
                            self.ans.net_dns.clone()
                        };
                        self.err.clear();
                    }
                    _ => {
                        if !dns_list_ok(&v) {
                            self.err = self
                                .t(
                                    "Invalid DNS list (space/comma-separated IPs).",
                                    "Liste DNS invalide (IP séparées par espace/virgule).",
                                )
                                .to_string();
                            return Action::None;
                        }
                        self.ans.net_dns = v;
                        self.err.clear();
                        self.next();
                    }
                }
            }
            KeyCode::Esc => {
                if self.field_idx == 0 {
                    return self.prev();
                }
                // Step back one sub-field, restoring its stored value.
                self.field_idx -= 1;
                self.input = match self.field_idx {
                    0 => self.ans.net_address.clone(),
                    _ => self.ans.net_gateway.clone(),
                };
                self.err.clear();
            }
            _ => {}
        }
        Action::None
    }

    /// Free-text entry for the hostname step; input is lowercased as typed
    /// since [`hostname_ok`] rejects uppercase.
    fn key_hostname(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char(c) => {
                self.input.push(c.to_ascii_lowercase());
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Enter => {
                if hostname_ok(&self.input) {
                    self.ans.hostname = self.input.trim().to_string();
                    self.next();
                } else {
                    self.err = self
                        .t(
                            "Invalid hostname (a-z, 0-9, hyphen; must start alphanumeric).",
                            "Nom d'hôte invalide (a-z, 0-9, tiret ; doit commencer par alphanumérique).",
                        )
                        .to_string();
                }
            }
            KeyCode::Esc => return self.prev(),
            _ => {}
        }
        Action::None
    }

    /// Two-phase entry (enter, then confirm) shared by `RootPwd` and
    /// `AdminPwd`. On mismatch, both buffers are cleared and phase resets to
    /// `Enter` so the user retypes from scratch rather than just the confirmation.
    fn key_password(&mut self, key: KeyEvent) -> Action {
        let buf = if self.pw_phase == PwPhase::Enter {
            &mut self.input
        } else {
            &mut self.input2
        };
        match key.code {
            KeyCode::Char(c) => buf.push(c),
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Enter => match self.pw_phase {
                PwPhase::Enter => {
                    if password_ok(&self.input) {
                        self.pw_phase = PwPhase::Confirm;
                        self.err.clear();
                    } else {
                        self.err = self
                            .t(
                                "Password too weak: min 12 chars, ≥1 letter and ≥1 digit.",
                                "Mot de passe trop faible : min 12 caractères, ≥1 lettre et ≥1 chiffre.",
                            )
                            .to_string();
                    }
                }
                PwPhase::Confirm => {
                    if self.input == self.input2 {
                        let pw = self.input.clone();
                        if self.step == Step::RootPwd {
                            self.ans.root_password = pw;
                        } else {
                            self.ans.admin_password = pw;
                        }
                        self.next();
                    } else {
                        self.err = self
                            .t("Passwords do not match.", "Les mots de passe ne correspondent pas.")
                            .to_string();
                        self.pw_phase = PwPhase::Enter;
                        self.input.clear();
                        self.input2.clear();
                    }
                }
            },
            KeyCode::Esc => {
                if self.pw_phase == PwPhase::Confirm {
                    self.pw_phase = PwPhase::Enter;
                    self.input2.clear();
                    self.err.clear();
                } else {
                    return self.prev();
                }
            }
            _ => {}
        }
        Action::None
    }

    fn key_confirm(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => self.start_install(),
            KeyCode::Esc => return self.prev(),
            _ => {}
        }
        Action::None
    }

    /// Write the answers file and launch the bash backend. Any failure here
    /// (can't write the conf, can't spawn the process) goes straight to the
    /// `Failed` screen since there is nothing left to retry from the wizard.
    fn start_install(&mut self) {
        if let Err(e) = write_conf(&self.ans) {
            self.fail_msg = format!("{e:#}");
            self.enter_step(Step::Failed);
            return;
        }
        match spawn_backend() {
            Ok(child) => {
                self.child = Some(child);
                self.progress = None;
                self.tick = 0;
                self.step = Step::Installing;
            }
            Err(e) => {
                self.fail_msg = format!("{e:#}");
                self.enter_step(Step::Failed);
            }
        }
    }

    /// Called by the event loop on every tick while installing.
    pub fn poll_install(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if let Some(p) = read_progress() {
            let status = p.status.clone();
            let detail = p.detail.clone();
            self.progress = Some(p);
            match status.as_str() {
                "done" => self.enter_step(Step::Done),
                "failed" => {
                    self.fail_msg = detail;
                    self.enter_step(Step::Failed);
                }
                _ => {}
            }
        }
        // Defensive: backend died without publishing a terminal status.
        if let Some(child) = self.child.as_mut() {
            if let Ok(Some(exit)) = child.try_wait() {
                self.child = None;
                if self.step == Step::Installing {
                    if exit.success() {
                        self.enter_step(Step::Done);
                    } else {
                        if self.fail_msg.is_empty() {
                            self.fail_msg = self
                                .t(
                                    "The backend exited unexpectedly. See /var/log/virtuaos-install.log",
                                    "Le backend s'est arrêté de façon inattendue. Voir /var/log/virtuaos-install.log",
                                )
                                .to_string();
                        }
                        self.enter_step(Step::Failed);
                    }
                }
            }
        }
    }

    // ── rendering ─────────────────────────────────────────────────────────────
    // Every step is drawn inside the same outer bordered box: a breadcrumb row
    // on top, the step-specific content in the middle (`draw_content`
    // dispatches to one `draw_*` method per step/group of steps below), and a
    // one-line key hint at the bottom.

    pub fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::title())
            .style(theme::base())
            .title(Span::styled(
                "  VirtuaOS · Installation  ",
                theme::title(),
            ))
            .title_alignment(Alignment::Center);
        let inner = outer.inner(area);
        f.render_widget(outer, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1), // breadcrumb
                Constraint::Length(1), // spacer
                Constraint::Min(3),    // content
                Constraint::Length(1), // hint
            ])
            .split(inner);

        self.draw_breadcrumb(f, rows[0]);
        self.draw_content(f, rows[2]);
        self.draw_hint(f, rows[3]);
    }

    /// Top breadcrumb: 7 coarse-grained groups (finer than [`Step`] — e.g.
    /// all network sub-steps collapse into one "Network" entry), highlighted
    /// by [`App::group_index`] to show progress at a glance.
    fn draw_breadcrumb(&self, f: &mut Frame, area: Rect) {
        let groups: [(Step, &str, &str); 7] = [
            (Step::UiLang, "Langue", "Language"),
            (Step::Keyboard, "Clavier", "Keyboard"),
            (Step::Hostname, "Serveur", "Server"),
            (Step::NetIface, "Réseau", "Network"),
            (Step::RootPwd, "Comptes", "Accounts"),
            (Step::Disk, "Disque", "Disk"),
            (Step::Installing, "Installation", "Install"),
        ];
        let active = self.group_index();
        let mut spans: Vec<Span> = Vec::new();
        for (i, g) in groups.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ·  ", theme::dim()));
            }
            let label = if self.fr() { g.1 } else { g.2 };
            let style = if i == active {
                theme::highlight()
            } else if i < active {
                theme::ok()
            } else {
                theme::dim()
            };
            spans.push(Span::styled(format!(" {} ", label), style));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
            area,
        );
    }

    fn group_index(&self) -> usize {
        match self.step {
            Step::UiLang | Step::Locale => 0,
            Step::Keyboard => 1,
            Step::Hostname | Step::Timezone => 2,
            Step::NetIface | Step::WifiScan | Step::WifiPass | Step::NetMethod | Step::NetStatic => 3,
            Step::RootPwd | Step::AdminPwd => 4,
            Step::Disk => 5,
            _ => 6,
        }
    }

    /// Route to the step-specific drawing function.
    fn draw_content(&mut self, f: &mut Frame, area: Rect) {
        match self.step {
            Step::UiLang | Step::Locale | Step::Keyboard | Step::Timezone | Step::NetMethod => {
                self.draw_menu(f, area)
            }
            Step::Disk => self.draw_disk(f, area),
            Step::NetIface => self.draw_net_iface(f, area),
            Step::WifiScan => self.draw_wifi_scan(f, area),
            Step::WifiPass => self.draw_wifi_pass(f, area),
            Step::NetStatic => self.draw_static(f, area),
            Step::Hostname => self.draw_hostname(f, area),
            Step::RootPwd | Step::AdminPwd => self.draw_password(f, area),
            Step::Confirm => self.draw_confirm(f, area),
            Step::Installing => self.draw_installing(f, area),
            Step::Done => self.draw_done(f, area),
            Step::Failed => self.draw_failed(f, area),
        }
    }

    fn section_block(&self, title: &str) -> Block<'static> {
        Block::default()
            .borders(Borders::NONE)
            .title(Span::styled(title.to_string(), theme::title()))
    }

    fn draw_menu(&mut self, f: &mut Frame, area: Rect) {
        let title = match self.step {
            Step::UiLang => self.t("Installer language", "Langue de l'installateur"),
            Step::Locale => self.t("System language", "Langue du système"),
            Step::Keyboard => self.t("Keyboard layout", "Disposition du clavier"),
            Step::Timezone => self.t("Timezone", "Fuseau horaire"),
            Step::NetMethod => self.t("Addressing", "Attribution de l'adresse"),
            _ => "",
        };
        let items: Vec<ListItem> = self
            .menu()
            .iter()
            .map(|c| ListItem::new(Line::from(c.label)))
            .collect();
        let list = List::new(items)
            .block(self.section_block(title))
            .highlight_style(theme::highlight())
            .highlight_symbol("▸ ");
        let mut state = ListState::default();
        state.select(Some(self.sel));
        f.render_stateful_widget(list, area, &mut state);
    }

    fn draw_disk(&mut self, f: &mut Frame, area: Rect) {
        if self.disks.is_empty() {
            let msg = self.t(
                "No internal disk detected. USB media is filtered out. Press Esc to go back.",
                "Aucun disque interne détecté. Les clés USB sont filtrées. Esc pour revenir.",
            );
            f.render_widget(
                Paragraph::new(msg)
                    .style(theme::warn())
                    .wrap(Wrap { trim: true })
                    .block(self.section_block(self.t("Target disk", "Disque cible"))),
                area,
            );
            return;
        }
        let items: Vec<ListItem> = self
            .disks
            .iter()
            .map(|d| {
                ListItem::new(Line::from(format!(
                    "{:<12} {:>9}   {}",
                    d.dev, d.size, d.model
                )))
            })
            .collect();
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(2)])
            .split(area);
        let list = List::new(items)
            .block(self.section_block(self.t("Target disk", "Disque cible")))
            .highlight_style(theme::highlight())
            .highlight_symbol("▸ ");
        let mut state = ListState::default();
        state.select(Some(self.sel));
        f.render_stateful_widget(list, rows[0], &mut state);
        f.render_widget(
            Paragraph::new(self.t(
                "⚠ The selected disk will be COMPLETELY ERASED.",
                "⚠ Le disque sélectionné sera COMPLÈTEMENT EFFACÉ.",
            ))
            .style(theme::warn())
            .wrap(Wrap { trim: true }),
            rows[1],
        );
    }

    fn draw_net_iface(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .ifaces
            .iter()
            .map(|i| {
                let icon = if i.is_wifi() { "📶" } else { "🔌" };
                let kind = if i.is_wifi() {
                    self.t("WiFi", "WiFi")
                } else {
                    self.t("Ethernet", "Filaire")
                };
                let state = if i.state.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", i.state)
                };
                ListItem::new(Line::from(format!(
                    "{icon}  {:<12} {:<10}{}",
                    i.dev, kind, state
                )))
            })
            .collect();
        let list = List::new(items)
            .block(self.section_block(self.t("Network interface", "Interface réseau")))
            .highlight_style(theme::highlight())
            .highlight_symbol("▸ ");
        let mut state = ListState::default();
        state.select(Some(self.sel));
        f.render_stateful_widget(list, area, &mut state);
    }

    fn draw_wifi_scan(&mut self, f: &mut Frame, area: Rect) {
        if self.wifis.is_empty() {
            f.render_widget(
                Paragraph::new(self.t(
                    "No WiFi network found. Press R to rescan, or Esc to go back.",
                    "Aucun réseau WiFi trouvé. Appuyez sur R pour re-scanner, ou Esc pour revenir.",
                ))
                .style(theme::warn())
                .wrap(Wrap { trim: true })
                .block(self.section_block(self.t("WiFi networks", "Réseaux WiFi"))),
                area,
            );
            return;
        }
        let items: Vec<ListItem> = self
            .wifis
            .iter()
            .map(|w| {
                let lock = if w.secured { "🔒" } else { "  " };
                ListItem::new(Line::from(format!(
                    "{lock} {:<28} {:>3}%",
                    w.ssid, w.signal
                )))
            })
            .collect();
        let list = List::new(items)
            .block(self.section_block(self.t("WiFi networks", "Réseaux WiFi")))
            .highlight_style(theme::highlight())
            .highlight_symbol("▸ ");
        let mut state = ListState::default();
        state.select(Some(self.sel));
        f.render_stateful_widget(list, area, &mut state);
    }

    fn draw_wifi_pass(&mut self, f: &mut Frame, area: Rect) {
        let dots: String = "•".repeat(self.input.chars().count());
        let mut lines = vec![
            Line::from(vec![
                Span::styled(self.t("Network: ", "Réseau : "), theme::dim()),
                Span::styled(self.ans.wifi_ssid.clone(), theme::base()),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                self.t("Enter the WiFi password:", "Entrez le mot de passe WiFi :"),
                theme::base(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("  ", theme::base()),
                Span::styled(format!("{}_", dots), theme::highlight()),
            ]),
        ];
        if !self.err.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(self.err.clone(), theme::error())));
        }
        f.render_widget(
            Paragraph::new(lines).block(self.section_block(self.t("WiFi password", "Mot de passe WiFi"))),
            area,
        );
    }

    fn draw_static(&mut self, f: &mut Frame, area: Rect) {
        // Show the three fields; the active one carries the live input + cursor.
        let labels = [
            self.t("IP address / prefix", "Adresse IP / préfixe"),
            self.t("Gateway", "Passerelle"),
            self.t("DNS servers", "Serveurs DNS"),
        ];
        let stored = [
            self.ans.net_address.clone(),
            self.ans.net_gateway.clone(),
            self.ans.net_dns.clone(),
        ];
        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            self.t(
                "Enter the static network settings:",
                "Entrez les paramètres réseau fixes :",
            ),
            theme::base(),
        )), Line::from("")];
        for i in 0..3 {
            let value = if i == self.field_idx {
                format!("{}_", self.input)
            } else if i < self.field_idx {
                stored[i].clone()
            } else {
                String::new()
            };
            let lbl_style = if i == self.field_idx {
                theme::highlight()
            } else {
                theme::dim()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<22}", labels[i]), lbl_style),
                Span::styled(value, theme::base()),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            self.t(
                "Examples: 192.168.1.50/24 · gateway 192.168.1.1 · DNS \"1.1.1.1 8.8.8.8\". Gateway/DNS optional.",
                "Exemples : 192.168.1.50/24 · passerelle 192.168.1.1 · DNS « 1.1.1.1 8.8.8.8 ». Passerelle/DNS optionnels.",
            ),
            theme::dim(),
        )));
        if !self.err.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(self.err.clone(), theme::error())));
        }
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .block(self.section_block(self.t("Static IP", "IP fixe"))),
            area,
        );
    }

    fn draw_hostname(&mut self, f: &mut Frame, area: Rect) {
        let mut lines = vec![
            Line::from(Span::styled(
                self.t("Enter the server hostname:", "Entrez le nom d'hôte du serveur :"),
                theme::base(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("  ", theme::base()),
                Span::styled(format!("{}_", self.input), theme::highlight()),
            ]),
        ];
        if !self.err.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(self.err.clone(), theme::error())));
        }
        f.render_widget(
            Paragraph::new(lines).block(self.section_block(self.t("Server name", "Nom du serveur"))),
            area,
        );
    }

    fn draw_password(&mut self, f: &mut Frame, area: Rect) {
        let title = if self.step == Step::RootPwd {
            self.t("Root password (shell)", "Mot de passe root (shell)")
        } else {
            self.t("Admin password (web panel)", "Mot de passe admin (panneau web)")
        };
        let phase_label = if self.pw_phase == PwPhase::Enter {
            self.t("Enter password:", "Entrez le mot de passe :")
        } else {
            self.t("Confirm password:", "Confirmez le mot de passe :")
        };
        let shown = if self.pw_phase == PwPhase::Enter {
            &self.input
        } else {
            &self.input2
        };
        let dots: String = "•".repeat(shown.chars().count());
        let mut lines = vec![
            Line::from(Span::styled(phase_label, theme::base())),
            Line::from(""),
            Line::from(vec![
                Span::styled("  ", theme::base()),
                Span::styled(format!("{}_", dots), theme::highlight()),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                self.t(
                    "Policy: ≥12 chars, at least one letter and one digit.",
                    "Politique : ≥12 caractères, au moins une lettre et un chiffre.",
                ),
                theme::dim(),
            )),
        ];
        if !self.err.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(self.err.clone(), theme::error())));
        }
        f.render_widget(
            Paragraph::new(lines).block(self.section_block(title)),
            area,
        );
    }

    /// One-line network summary for the confirmation screen.
    fn net_summary(&self) -> String {
        if self.ans.net_iface.is_empty() {
            return self.t("auto (DHCP on default link)", "auto (DHCP sur le lien par défaut)").to_string();
        }
        let mut s = self.ans.net_iface.clone();
        if self.ans.net_kind == "wifi" && !self.ans.wifi_ssid.is_empty() {
            s.push_str(&format!(" · WiFi \"{}\"", self.ans.wifi_ssid));
        }
        if self.ans.net_method == "static" {
            s.push_str(&format!(" · {}", self.ans.net_address));
            if !self.ans.net_gateway.is_empty() {
                s.push_str(&format!(" gw {}", self.ans.net_gateway));
            }
        } else {
            s.push_str(" · DHCP");
        }
        s
    }

    fn draw_confirm(&mut self, f: &mut Frame, area: Rect) {
        let row = |k: &str, v: &str| {
            Line::from(vec![
                Span::styled(format!("  {:<12}", k), theme::dim()),
                Span::styled(v.to_string(), theme::base()),
            ])
        };
        let lines = vec![
            Line::from(Span::styled(
                self.t(
                    "Review your choices, then press Enter to install:",
                    "Vérifiez vos choix, puis Entrée pour installer :",
                ),
                theme::base(),
            )),
            Line::from(""),
            row(self.t("Disk", "Disque"), &self.ans.disk),
            row(self.t("Hostname", "Nom d'hôte"), &self.ans.hostname),
            row(self.t("Language", "Langue"), &self.ans.lang_code),
            row(self.t("Keyboard", "Clavier"), &self.ans.keymap),
            row(self.t("Timezone", "Fuseau"), &self.ans.tz),
            row(self.t("Network", "Réseau"), &self.net_summary()),
            row(self.t("Shell user", "User shell"), "virtua"),
            row(self.t("Web admin", "Admin web"), "admin"),
            Line::from(""),
            Line::from(Span::styled(
                self.t(
                    "⚠ FINAL WARNING — all data on the disk will be lost.",
                    "⚠ DERNIER AVERTISSEMENT — toutes les données du disque seront perdues.",
                ),
                theme::warn(),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .block(self.section_block(self.t("Summary", "Récapitulatif"))),
            area,
        );
    }

    /// Progress gauge + spinner driven by [`App::poll_install`]'s polling of
    /// the backend's progress file (via `self.progress`) and `self.tick`.
    fn draw_installing(&mut self, f: &mut Frame, area: Rect) {
        let p = self.progress.clone().unwrap_or_default();
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // current step label
                Constraint::Length(1), // spacer
                Constraint::Length(3), // gauge
                Constraint::Length(1), // spacer
                Constraint::Min(1),    // detail
            ])
            .split(area);

        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let s = spinner[(self.tick as usize) % spinner.len()];
        let label = if p.label.is_empty() {
            self.t("Starting…", "Démarrage…").to_string()
        } else {
            p.label.clone()
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {s} "), theme::title()),
                Span::styled(label, theme::base()),
            ])),
            rows[0],
        );

        let gauge = Gauge::default()
            .gauge_style(theme::highlight())
            .ratio((p.pct as f64 / 100.0).clamp(0.0, 1.0))
            .label(format!("{} %", p.pct));
        f.render_widget(gauge, rows[2]);

        if !p.detail.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled(p.detail.clone(), theme::dim()))
                    .wrap(Wrap { trim: true }),
                rows[4],
            );
        }
    }

    fn draw_done(&mut self, f: &mut Frame, area: Rect) {
        let lines = vec![
            Line::from(Span::styled(
                self.t("✓ VirtuaOS is installed!", "✓ VirtuaOS est installé !"),
                theme::ok(),
            )),
            Line::from(""),
            Line::from(self.t(
                "After reboot, reach the panel at  http://<IP>:8441",
                "Au redémarrage, le panneau est sur  http://<IP>:8441",
            )),
            Line::from(self.t(
                "  Shell: virtua · Root: root · Web: admin (passwords you set)",
                "  Shell : virtua · Root : root · Web : admin (mots de passe choisis)",
            )),
            Line::from(""),
            Line::from(Span::styled(
                self.t(
                    "IMPORTANT: remove the USB install media before rebooting.",
                    "IMPORTANT : retirez la clé USB d'installation avant de redémarrer.",
                ),
                theme::warn(),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .block(self.section_block(self.t("Installation complete", "Installation terminée"))),
            area,
        );
    }

    fn draw_failed(&mut self, f: &mut Frame, area: Rect) {
        let mut lines = vec![
            Line::from(Span::styled(
                self.t("✗ Installation failed", "✗ L'installation a échoué"),
                theme::error(),
            )),
            Line::from(""),
        ];
        if !self.fail_msg.is_empty() {
            lines.push(Line::from(Span::styled(self.fail_msg.clone(), theme::base())));
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            self.t(
                "Log: /var/log/virtuaos-install.log",
                "Journal : /var/log/virtuaos-install.log",
            ),
            theme::dim(),
        )));
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .block(self.section_block(self.t("Installation failed", "Échec de l'installation"))),
            area,
        );
    }

    fn draw_hint(&self, f: &mut Frame, area: Rect) {
        let hint = match self.step {
            Step::UiLang | Step::Locale | Step::Keyboard | Step::Timezone | Step::Disk
            | Step::NetIface | Step::NetMethod => self.t(
                "↑↓ choose   ⏎ continue   Esc back",
                "↑↓ choisir   ⏎ continuer   Esc retour",
            ),
            Step::WifiScan => self.t(
                "↑↓ choose   ⏎ connect   R rescan   Esc back",
                "↑↓ choisir   ⏎ connecter   R re-scanner   Esc retour",
            ),
            Step::WifiPass | Step::NetStatic => self.t(
                "type · ⏎ continue · Esc back",
                "tapez · ⏎ continuer · Esc retour",
            ),
            Step::Hostname => self.t("type · ⏎ continue · Esc back", "tapez · ⏎ continuer · Esc retour"),
            Step::RootPwd | Step::AdminPwd => self.t(
                "type · ⏎ continue · Esc back",
                "tapez · ⏎ continuer · Esc retour",
            ),
            Step::Confirm => self.t(
                "⏎ start installation   Esc back",
                "⏎ lancer l'installation   Esc retour",
            ),
            Step::Installing => self.t("Installing — please wait…", "Installation — patientez…"),
            Step::Done => self.t("R reboot now   Q stay in live mode", "R redémarrer   Q rester en live"),
            Step::Failed => self.t("any key → shell", "n'importe quelle touche → shell"),
        };
        f.render_widget(
            Paragraph::new(Span::styled(hint, theme::dim())).alignment(Alignment::Center),
            area,
        );
    }
}
