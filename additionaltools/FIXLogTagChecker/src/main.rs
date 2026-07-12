use eframe::egui;
use std::collections::HashMap;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 700.0])
            .with_title("FIX Log Checker"),
        ..Default::default()
    };

    eframe::run_native(
        "FIX Log Checker",
        options,
        Box::new(|_cc| Ok(Box::new(FixLogChecker::default()))),
    )
}

#[derive(PartialEq)]
enum Tab {
    MessageParser,
    LogAnalyzer,
}

struct FixLogChecker {
    current_tab: Tab,

    // Tab 1: Single message parser
    fix_message: String,
    parsed_tags: Vec<ParsedTag>,
    parser_error: Option<String>,

    // Tab 2: Log analyzer
    fix_log: String,
    target_tag: String,
    extracted_values: Vec<String>,
    analyzer_error: Option<String>,
}

#[derive(Clone)]
struct ParsedTag {
    tag: String,
    name: String,
    value: String,
}

impl Default for FixLogChecker {
    fn default() -> Self {
        Self {
            current_tab: Tab::MessageParser,
            fix_message: String::new(),
            parsed_tags: Vec::new(),
            parser_error: None,
            fix_log: String::new(),
            target_tag: String::new(),
            extracted_values: Vec::new(),
            analyzer_error: None,
        }
    }
}

impl eframe::App for FixLogChecker {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("FIX Log Checker");
            ui.add_space(10.0);

            // Browser-style tabs
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;

                // Message Parser Tab
                let parser_selected = self.current_tab == Tab::MessageParser;
                let parser_response = ui.add(
                    egui::Button::new(egui::RichText::new("  Message Parser  ").size(14.0))
                        .fill(if parser_selected {
                            ctx.style().visuals.extreme_bg_color
                        } else {
                            ui.visuals().widgets.inactive.weak_bg_fill
                        })
                        .stroke(if parser_selected {
                            egui::Stroke::NONE
                        } else {
                            egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color)
                        })
                        .rounding(egui::Rounding {
                            nw: 4.0,
                            ne: 4.0,
                            sw: 0.0,
                            se: 0.0,
                        }),
                );

                if parser_response.clicked() {
                    self.current_tab = Tab::MessageParser;
                }

                ui.add_space(2.0);

                // Log Analyzer Tab
                let analyzer_selected = self.current_tab == Tab::LogAnalyzer;
                let analyzer_response = ui.add(
                    egui::Button::new(egui::RichText::new("  Log Analyzer  ").size(14.0))
                        .fill(if analyzer_selected {
                            ctx.style().visuals.extreme_bg_color
                        } else {
                            ui.visuals().widgets.inactive.weak_bg_fill
                        })
                        .stroke(if analyzer_selected {
                            egui::Stroke::NONE
                        } else {
                            egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color)
                        })
                        .rounding(egui::Rounding {
                            nw: 4.0,
                            ne: 4.0,
                            sw: 0.0,
                            se: 0.0,
                        }),
                );

                if analyzer_response.clicked() {
                    self.current_tab = Tab::LogAnalyzer;
                }
            });

            ui.add_space(5.0);

            egui::ScrollArea::vertical().show(ui, |ui| match self.current_tab {
                Tab::MessageParser => self.show_parser_tab(ui),
                Tab::LogAnalyzer => self.show_analyzer_tab(ui),
            });
        });
    }
}

impl FixLogChecker {
    fn show_parser_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Parse FIX Message");
        ui.add_space(5.0);

        ui.label("Paste a FIX message (SOH-delimited or pipe-delimited):");

        let response = ui.add_sized(
            [ui.available_width(), 120.0],
            egui::TextEdit::multiline(&mut self.fix_message)
                .hint_text("8=FIX.4.2|9=178|35=D|49=SENDER|56=TARGET|34=1|52=20240101-12:00:00|11=ORDER123|...")
        );

        if response.changed() {
            self.parser_error = None;
            if !self.fix_message.is_empty() {
                self.parsed_tags = parse_fix_message(&self.fix_message);
            }
        }

        if ui.button("Parse Message").clicked() {
            if self.fix_message.trim().is_empty() {
                self.parser_error = Some("Please input message".to_string());
                self.parsed_tags.clear();
            } else {
                self.parser_error = None;
                self.parsed_tags = parse_fix_message(&self.fix_message);
            }
        }

        // Show error message in red if present
        if let Some(error) = &self.parser_error {
            ui.add_space(5.0);
            ui.label(egui::RichText::new(error).color(egui::Color32::RED));
        }

        ui.add_space(10.0);

        if !self.parsed_tags.is_empty() {
            ui.separator();
            ui.add_space(10.0);
            ui.heading("Parsed Tags");
            ui.add_space(5.0);

            egui::Grid::new("parsed_tags_grid")
                .striped(true)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Tag").strong());
                    ui.label(egui::RichText::new("Name").strong());
                    ui.label(egui::RichText::new("Value").strong());
                    ui.end_row();

                    for tag in &self.parsed_tags {
                        ui.label(&tag.tag);
                        ui.label(&tag.name);
                        ui.label(&tag.value);
                        ui.end_row();
                    }
                });
        }
    }

    fn show_analyzer_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Extract Tag Values from Log");
        ui.add_space(5.0);

        ui.horizontal(|ui| {
            ui.label("Target Tag:");
            ui.add(
                egui::TextEdit::singleline(&mut self.target_tag)
                    .hint_text("e.g., 11, 48, 55")
                    .desired_width(100.0),
            );
        });

        ui.add_space(5.0);

        ui.label("Upload FIX log file:");
        ui.horizontal(|ui| {
            if ui.button("Choose File...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Log Files", &["log", "txt"])
                    .add_filter("All Files", &["*"])
                    .pick_file()
                {
                    match std::fs::read_to_string(&path) {
                        Ok(contents) => {
                            self.fix_log = contents;
                            self.analyzer_error = None;
                        }
                        Err(e) => {
                            self.analyzer_error = Some(format!("Error reading file: {}", e));
                        }
                    }
                }
            }

            if !self.fix_log.is_empty() {
                ui.label(format!("Loaded {} bytes", self.fix_log.len()));
            }
        });

        ui.add_space(5.0);

        if ui.button("Extract Values").clicked() {
            if self.target_tag.trim().is_empty() {
                self.analyzer_error = Some("Please enter a target tag".to_string());
                self.extracted_values.clear();
            } else if self.fix_log.trim().is_empty() {
                self.analyzer_error = Some("Please upload a log file".to_string());
                self.extracted_values.clear();
            } else {
                self.analyzer_error = None;
                self.extracted_values = extract_tag_values(&self.fix_log, &self.target_tag);
            }
        }

        // Show error message in red if present
        if let Some(error) = &self.analyzer_error {
            ui.add_space(5.0);
            ui.label(egui::RichText::new(error).color(egui::Color32::RED));
        }

        ui.add_space(10.0);

        if !self.extracted_values.is_empty() {
            ui.separator();
            ui.add_space(10.0);
            ui.heading(format!(
                "Values for Tag {} ({}):",
                self.target_tag,
                get_tag_name(&self.target_tag)
            ));
            ui.add_space(5.0);

            egui::Grid::new("extracted_values_grid")
                .striped(true)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("#").strong());
                    ui.label(egui::RichText::new("Value").strong());
                    ui.end_row();

                    for (i, value) in self.extracted_values.iter().enumerate() {
                        ui.label(format!("{}", i + 1));
                        ui.label(value);
                        ui.end_row();
                    }
                });

            ui.add_space(10.0);
            ui.label(format!(
                "Total occurrences: {}",
                self.extracted_values.len()
            ));
        } else if !self.fix_log.is_empty()
            && !self.target_tag.is_empty()
            && self.analyzer_error.is_none()
        {
            ui.label("No values found for the specified tag.");
        }
    }
}

fn parse_fix_message(message: &str) -> Vec<ParsedTag> {
    let mut tags = Vec::new();

    // Replace common delimiters with SOH for uniform parsing
    let normalized = message.replace('|', "\x01").replace('^', "\x01");

    for pair in normalized.split('\x01') {
        if let Some((tag, value)) = pair.split_once('=') {
            let tag = tag.trim();
            let value = value.trim();

            if !tag.is_empty() {
                tags.push(ParsedTag {
                    tag: tag.to_string(),
                    name: get_tag_name(tag),
                    value: value.to_string(),
                });
            }
        }
    }

    tags
}

fn extract_tag_values(log: &str, target_tag: &str) -> Vec<String> {
    let mut values = Vec::new();

    // Normalize delimiters
    let normalized = log.replace('|', "\x01").replace('^', "\x01");

    let search_pattern = format!("{}=", target_tag);

    for pair in normalized.split('\x01') {
        if pair.trim().starts_with(&search_pattern) {
            if let Some((_, value)) = pair.split_once('=') {
                let value = value.trim();
                if !value.is_empty() {
                    values.push(value.to_string());
                }
            }
        }
    }

    values
}

fn get_tag_name(tag: &str) -> String {
    // Common FIX tags mapping
    let tag_map: HashMap<&str, &str> = [
        ("1", "Account"),
        ("6", "AvgPx"),
        ("8", "BeginString"),
        ("9", "BodyLength"),
        ("10", "CheckSum"),
        ("11", "ClOrdID"),
        ("14", "CumQty"),
        ("15", "Currency"),
        ("17", "ExecID"),
        ("18", "ExecInst"),
        ("20", "ExecTransType"),
        ("21", "HandlInst"),
        ("22", "SecurityIDSource"),
        ("30", "LastMkt"),
        ("31", "LastPx"),
        ("32", "LastQty"),
        ("34", "MsgSeqNum"),
        ("35", "MsgType"),
        ("37", "OrderID"),
        ("38", "OrderQty"),
        ("39", "OrdStatus"),
        ("40", "OrdType"),
        ("41", "OrigClOrdID"),
        ("43", "PossDupFlag"),
        ("44", "Price"),
        ("47", "Rule80A"),
        ("48", "SecurityID"),
        ("49", "SenderCompID"),
        ("50", "SenderSubID"),
        ("52", "SendingTime"),
        ("54", "Side"),
        ("55", "Symbol"),
        ("56", "TargetCompID"),
        ("57", "TargetSubID"),
        ("58", "Text"),
        ("59", "TimeInForce"),
        ("60", "TransactTime"),
        ("63", "SettlType"),
        ("64", "SettlDate"),
        ("76", "ExecBroker"),
        ("100", "ExDestination"),
        ("103", "OrdRejReason"),
        ("108", "HeartBtInt"),
        ("109", "ClientID"),
        ("110", "MinQty"),
        ("111", "MaxFloor"),
        ("114", "LocateReqd"),
        ("115", "OnBehalfOfCompID"),
        ("117", "QuoteID"),
        ("122", "OrigSendingTime"),
        ("123", "GapFillFlag"),
        ("126", "ExpireTime"),
        ("128", "DeliverToCompID"),
        ("141", "ResetSeqNumFlag"),
        ("142", "SenderLocationID"),
        ("143", "TargetLocationID"),
        ("150", "ExecType"),
        ("151", "LeavesQty"),
        ("167", "SecurityType"),
        ("200", "MaturityMonthYear"),
        ("207", "SecurityExchange"),
        ("336", "TradingSessionID"),
        ("378", "ExecRestatementReason"),
        ("432", "ExpireDate"),
        ("439", "ClearingFirm"),
        ("447", "PartyIDSource"),
        ("448", "PartyID"),
        ("452", "PartyRole"),
        ("528", "OrderCapacity"),
        ("581", "AccountType"),
    ]
    .iter()
    .cloned()
    .collect();

    tag_map.get(tag).unwrap_or(&"Unknown").to_string()
}
