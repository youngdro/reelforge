//! The agents window: whether AI agents (and other programs) can edit this project, where they
//! connect, and what to paste into Claude Code, Claude Desktop or any other MCP client to connect
//! them. On the desktop it also starts sharing: the app hands the editor an [`AgentHost`] that
//! puts the project's session behind an MCP server, so nothing needs a command line. Opened from
//! the agent button in the top bar or the command palette.

use eframe::egui;

use crate::{EditorApp, theme};

/// Starts serving this project to agents at an address (`127.0.0.1:7878`), with a token or
/// none; answers the base address clients use (`http://192.168.1.4:7878`), or why it could not.
pub type AgentHost = Box<dyn FnMut(&str, Option<String>) -> Result<String, String>>;

/// The port asked for first.
const PORT: &str = "7878";

/// The window's state.
pub(crate) struct AgentsWindow {
    pub(crate) open: bool,
    /// Where agents reach this project (`http://127.0.0.1:7878`) and the token they send.
    pub(crate) serving: Option<(String, Option<String>)>,
    /// Where other computers reach it (`http://192.168.1.4:7878`), when they may.
    pub(crate) lan: Option<String>,
    pub(crate) host: Option<AgentHost>,
    /// The port being typed.
    pub(crate) port: String,
    /// Reachable from other computers (with a token).
    pub(crate) others: bool,
    pub(crate) token: String,
    pub(crate) error: Option<String>,
}

impl Default for AgentsWindow {
    fn default() -> AgentsWindow {
        AgentsWindow {
            open: false,
            serving: None,
            lan: None,
            host: None,
            port: PORT.into(),
            others: false,
            token: String::new(),
            error: None,
        }
    }
}

/// A token hard to guess: 24 hex digits from the clock and the window's own address.
fn new_token() -> String {
    use sha2::{Digest, Sha256};
    let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let seed = format!("{now}-{:p}", &now);
    Sha256::digest(seed.as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// Where other computers on the network reach a server on `port` here (`http://192.168.1.4:7878`):
/// this computer's address on the interface that leads out.
#[cfg(not(target_arch = "wasm32"))]
pub fn lan_base(port: u16) -> Option<String> {
    // connecting a UDP socket sends nothing; it only picks the interface that would be used
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then(|| format!("http://{}", std::net::SocketAddr::new(ip, port)))
}

#[cfg(target_arch = "wasm32")]
pub fn lan_base(_port: u16) -> Option<String> {
    None
}

/// What to paste where, for the server at `base` with `token`: (where it goes, the text).
pub(crate) fn snippets(base: &str, token: Option<&str>) -> Vec<(&'static str, String)> {
    let url = format!("{base}/mcp");
    let header = token.map(|t| format!("Authorization: Bearer {t}"));
    let code = match &header {
        Some(h) => format!("claude mcp add --transport http reelforge {url} --header \"{h}\""),
        None => format!("claude mcp add --transport http reelforge {url}"),
    };
    let mut http = serde_json::json!({"type": "http", "url": url});
    if let Some(t) = token {
        http["headers"] = serde_json::json!({"Authorization": format!("Bearer {t}")});
    }
    let mut args = vec!["-y".to_string(), "mcp-remote".into(), url.clone()];
    if let Some(h) = &header {
        args.extend(["--header".into(), h.clone()]);
    }
    let pretty = |v: serde_json::Value| serde_json::to_string_pretty(&v).expect("serialises");
    vec![
        ("claude", code),
        ("json", pretty(serde_json::json!({"mcpServers": {"reelforge": http}}))),
        ("desktop", pretty(serde_json::json!({"mcpServers": {"reelforge": {"command": "npx", "args": args}}}))),
    ]
}

impl EditorApp {
    /// Lets the editor share its project with agents itself (the desktop app passes this).
    pub fn with_agent_host(mut self, host: AgentHost) -> EditorApp {
        self.agents.host = Some(host);
        self
    }

    /// Says the project is already shared with agents at `base` (`http://127.0.0.1:7878`),
    /// as when the app was started with `--serve`.
    /// `lan` is where other computers reach it, when it listens beyond this one.
    pub fn with_agents_at(mut self, base: String, token: Option<String>, lan: Option<String>) -> EditorApp {
        self.agents.serving = Some((base, token));
        self.agents.lan = lan;
        self
    }

    /// Where agents reach this project: where it is served from here, or the server it was
    /// joined from.
    fn agents_at(&self) -> Option<(String, Option<String>)> {
        self.agents.serving.clone().or_else(|| self.remote.as_ref().map(|r| (r.base().to_string(), r.token())))
    }

    /// The agents window, while open.
    pub(crate) fn agents_window(&mut self, ctx: &egui::Context) {
        if !self.agents.open {
            return;
        }
        let s = self.s();
        let at = self.agents_at();
        let mut open = true;
        let mut start = false;
        egui::Window::new(s.agents_title)
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_TOP, [-12.0, 48.0])
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                let weak = |t: &str| egui::RichText::new(t.to_string()).color(theme::color::WEAK);
                ui.label(weak(s.agents_about));
                ui.add_space(6.0);
                match &at {
                    Some((base, token)) => {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(theme::icon::CONNECTED).color(theme::color::ACCENT));
                            ui.label(crate::i18n::fill(s.agents_serving, &[&format!("{base}/mcp")]));
                        });
                        if let Some(lan) = &self.agents.lan {
                            ui.label(weak(&crate::i18n::fill(s.agents_lan, &[&format!("{lan}/mcp")])));
                        }
                        if let Some(t) = token {
                            ui.horizontal(|ui| {
                                ui.label(weak(s.agents_token));
                                ui.monospace(t);
                                if ui.small_button(s.agents_copy).clicked() {
                                    ui.ctx().copy_text(t.clone());
                                }
                            });
                        }
                        ui.add_space(6.0);
                        for (kind, text) in snippets(base, token.as_deref()) {
                            let label = match kind {
                                "claude" => s.agents_claude_code,
                                "json" => s.agents_json,
                                _ => s.agents_desktop,
                            };
                            ui.horizontal(|ui| {
                                ui.label(weak(label));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button(s.agents_copy).clicked() {
                                        ui.ctx().copy_text(text.clone());
                                    }
                                });
                            });
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(egui::Label::new(egui::RichText::new(&text).monospace().size(11.0)).wrap());
                            });
                            ui.add_space(4.0);
                        }
                        ui.label(weak(s.agents_after));
                    }
                    None if self.agents.host.is_some() => {
                        let a = &mut self.agents;
                        ui.label(s.agents_off);
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(weak(s.agents_port));
                            ui.add(egui::TextEdit::singleline(&mut a.port).desired_width(64.0));
                        });
                        if ui.checkbox(&mut a.others, s.agents_others).on_hover_text(s.agents_others_tip).changed()
                            && a.others
                            && a.token.is_empty()
                        {
                            a.token = new_token();
                        }
                        if a.others {
                            ui.horizontal(|ui| {
                                ui.label(weak(s.agents_token));
                                ui.add(
                                    egui::TextEdit::singleline(&mut a.token)
                                        .desired_width(220.0)
                                        .font(egui::TextStyle::Monospace),
                                );
                            });
                        }
                        ui.add_space(6.0);
                        start = theme::primary(ui, theme::icon::AGENT, s.agents_start, true).clicked();
                        if let Some(e) = &a.error {
                            ui.label(egui::RichText::new(e).color(theme::color::WARN));
                        }
                    }
                    None => {
                        ui.label(s.agents_off);
                        ui.label(weak(s.agents_how));
                    }
                }
            });
        if start {
            self.start_sharing();
        }
        if !open {
            self.agents.open = false;
        }
    }

    /// Starts sharing with what the window holds: the port, and a token when other computers may
    /// connect.
    pub(crate) fn start_sharing(&mut self) {
        let s = self.s();
        let a = &mut self.agents;
        let port = a.port.trim().to_string();
        let token = (a.others && !a.token.trim().is_empty()).then(|| a.token.trim().to_string());
        let addr = if a.others { format!("0.0.0.0:{port}") } else { format!("127.0.0.1:{port}") };
        if port.parse::<u16>().is_err() {
            a.error = Some(s.agents_bad_port.into());
        } else if a.others && token.is_none() {
            a.error = Some(s.agents_need_token.into());
        } else if let Some(host) = a.host.as_mut() {
            match host(&addr, token.clone()) {
                Ok(base) => {
                    a.error = None;
                    a.lan = if a.others { port.parse().ok().and_then(lan_base) } else { None };
                    a.serving = Some((base, token));
                }
                Err(e) => a.error = Some(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_carry_the_address_and_the_token() {
        let open = snippets("http://127.0.0.1:7878", None);
        assert_eq!(open[0].1, "claude mcp add --transport http reelforge http://127.0.0.1:7878/mcp");
        assert!(!open.iter().any(|(_, t)| t.contains("Bearer")));
        let shut = snippets("http://192.168.1.4:7878", Some("abc"));
        assert!(shut[0].1.ends_with("--header \"Authorization: Bearer abc\""));
        let json: serde_json::Value = serde_json::from_str(&shut[1].1).unwrap();
        assert_eq!(json["mcpServers"]["reelforge"]["url"], "http://192.168.1.4:7878/mcp");
        assert_eq!(json["mcpServers"]["reelforge"]["headers"]["Authorization"], "Bearer abc");
        let desktop: serde_json::Value = serde_json::from_str(&shut[2].1).unwrap();
        let args = desktop["mcpServers"]["reelforge"]["args"].as_array().unwrap();
        assert_eq!(args[1], "mcp-remote");
        assert_eq!(args[4], "Authorization: Bearer abc");
        assert_eq!(new_token().len(), 24);
    }
}
