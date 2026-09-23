//! Optional Docker stack selection for Full setup (`d3`).

/// Which optional Docker stacks to start in `d3` (Homepage stays in `d2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct StackOpts {
    /// Dockge compose stack (`:5001`).
    pub dockge: bool,
    /// Open-WebUI (`:3000`).
    pub open_webui: bool,
    /// EVCC (`:7070`).
    pub evcc: bool,
    /// Whisper STT (`whisper-cv`, `:8000`).
    pub whisper: bool,
    /// `DeepSeek` / NPU LLM (`deepseek-npu`, `:8001`).
    pub deepseek: bool,
    /// Piper TTS (`:10200`).
    pub piper: bool,
    /// `OpenWakeWord` (`:10400`).
    pub openwakeword: bool,
}

/// One selected stack: compose directory name under the docker root + Services link label:port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackLink {
    /// Directory under `/srv/docker` (or dockge sibling for Dockge).
    pub dir: &'static str,
    /// `Name:port` fragment for `service_links.env` `LINKS`.
    pub link: &'static str,
}

impl StackOpts {
    /// No optional stacks selected.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            dockge: false,
            open_webui: false,
            evcc: false,
            whisper: false,
            deepseek: false,
            piper: false,
            openwakeword: false,
        }
    }

    /// Parse a comma-separated list (`dockge,open-webui,evcc,…`). Unknown tokens are ignored.
    #[must_use]
    pub fn parse_csv(raw: &str) -> Self {
        let mut opts = Self::none();
        for part in raw.split(',') {
            let key = part.trim().to_ascii_lowercase();
            if key.is_empty() {
                continue;
            }
            match key.as_str() {
                "dockge" => opts.dockge = true,
                "open-webui" | "openwebui" => opts.open_webui = true,
                "evcc" => opts.evcc = true,
                "whisper" | "whisper-cv" => opts.whisper = true,
                "deepseek" | "deepseek-npu" => opts.deepseek = true,
                "piper" => opts.piper = true,
                "openwakeword" | "open-wake-word" => opts.openwakeword = true,
                _ => {}
            }
        }
        opts
    }

    /// CSV of selected stack ids (stable order).
    #[must_use]
    pub fn to_csv(self) -> String {
        let mut parts = Vec::new();
        if self.dockge {
            parts.push("dockge");
        }
        if self.open_webui {
            parts.push("open-webui");
        }
        if self.evcc {
            parts.push("evcc");
        }
        if self.whisper {
            parts.push("whisper");
        }
        if self.deepseek {
            parts.push("deepseek");
        }
        if self.piper {
            parts.push("piper");
        }
        if self.openwakeword {
            parts.push("openwakeword");
        }
        parts.join(",")
    }

    /// True when at least one optional stack is selected.
    #[must_use]
    pub const fn any(self) -> bool {
        self.dockge
            || self.open_webui
            || self.evcc
            || self.whisper
            || self.deepseek
            || self.piper
            || self.openwakeword
    }

    /// Selected stacks in start order (Dockge first).
    #[must_use]
    pub fn selected(self) -> Vec<StackLink> {
        let mut out = Vec::new();
        if self.dockge {
            out.push(StackLink {
                dir: "dockge",
                link: "Dockge:5001",
            });
        }
        if self.open_webui {
            out.push(StackLink {
                dir: "open-webui",
                link: "Open-WebUI:3000",
            });
        }
        if self.evcc {
            out.push(StackLink {
                dir: "evcc",
                link: "EVCC:7070",
            });
        }
        if self.whisper {
            out.push(StackLink {
                dir: "whisper-cv",
                link: "Whisper:8000",
            });
        }
        if self.deepseek {
            out.push(StackLink {
                dir: "deepseek-npu",
                link: "DeepSeek:8001",
            });
        }
        if self.piper {
            out.push(StackLink {
                dir: "piper",
                link: "Piper:10200",
            });
        }
        if self.openwakeword {
            out.push(StackLink {
                dir: "openwakeword",
                link: "OpenWakeWord:10400",
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_csv_roundtrip() {
        let opts = StackOpts::parse_csv("dockge, Open-WebUI ,whisper-cv,deepseek-npu");
        assert!(opts.dockge);
        assert!(opts.open_webui);
        assert!(opts.whisper);
        assert!(opts.deepseek);
        assert!(!opts.evcc);
        assert_eq!(opts.to_csv(), "dockge,open-webui,whisper,deepseek");
    }

    #[test]
    fn parse_aliases_and_ignore_unknown() {
        let opts = StackOpts::parse_csv(
            "openwebui,evcc,piper,open-wake-word,openwakeword,deepseek,,bogus",
        );
        assert!(opts.open_webui);
        assert!(opts.evcc);
        assert!(opts.piper);
        assert!(opts.openwakeword);
        assert!(opts.deepseek);
        assert!(!opts.dockge);
        assert!(opts.any());
        assert_eq!(opts.to_csv(), "open-webui,evcc,deepseek,piper,openwakeword");
        assert!(!StackOpts::none().any());
        assert_eq!(StackOpts::parse_csv("").to_csv(), "");
    }

    #[test]
    fn selected_order_dockge_first() {
        let opts = StackOpts {
            dockge: true,
            piper: true,
            ..StackOpts::none()
        };
        let sel = opts.selected();
        assert_eq!(sel[0].dir, "dockge");
        assert_eq!(sel[1].dir, "piper");
    }

    #[test]
    fn selected_covers_all_optional_stacks() {
        let opts = StackOpts {
            dockge: true,
            open_webui: true,
            evcc: true,
            whisper: true,
            deepseek: true,
            piper: true,
            openwakeword: true,
        };
        let dirs: Vec<_> = opts.selected().iter().map(|s| s.dir).collect();
        assert_eq!(
            dirs,
            [
                "dockge",
                "open-webui",
                "evcc",
                "whisper-cv",
                "deepseek-npu",
                "piper",
                "openwakeword"
            ]
        );
    }
}
