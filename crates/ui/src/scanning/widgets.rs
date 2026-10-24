//! Drawing: the device row and the settings. The model and the state machine are in the parent module.

use super::*;

impl State {
    /// Draw just the device row and the settings. Apps render [`State::feedback`], the Scan action
    /// and everything else around it, and wrap this in `add_enabled_ui` while a scan runs.
    ///
    /// The first call starts a listing; selecting a scanner reads its details. Neither feeds paper
    /// or scans.
    pub fn show(&mut self, ui: &mut egui::Ui, id: egui::Id, labels: &Labels<'_>, discover: &Discover, load_choices: &LoadChoices) -> Response {
        if !self.requested {
            self.refresh(discover);
        }
        let before = self.settings.clone();
        self.poll(ui.ctx());
        let mut response = Response::default();
        ui.push_id(id, |ui| {
            self.device_row(ui, labels, discover, load_choices, &mut response);
            if let Some(choices) = self.choices() {
                self.controls(ui, labels, &choices);
            }
        });
        self.ensure_details(load_choices);
        response.changed = self.settings != before;
        self.repaint_if_pending(ui.ctx());
        response
    }

    /// Draws the scanner combo box, the refresh button, and the progress or retry control for the choices read.
    /// A refresh or retry that starts is recorded in `response`.
    fn device_row(&mut self, ui: &mut egui::Ui, labels: &Labels<'_>, discover: &Discover, load_choices: &LoadChoices, response: &mut Response) {
        ui.horizontal_wrapped(|ui| {
            let caption = ui.label(labels.scanner);
            let text = self.selected().map_or_else(|| labels.select_scanner.to_string(), |s| labels.device_label(s));
            let width = ui.available_width().clamp(32.0, 280.0);
            let scanners = &self.scanners;
            let selected = &mut self.settings.scanner;
            egui::ComboBox::from_id_salt("scanner")
                .width(width)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for scanner in scanners {
                        let mut entry = ui.selectable_label(*selected == scanner.id, labels.device_label(scanner));
                        if let Some(note) = same_printer_note(scanners, scanner) {
                            entry = entry.on_hover_text(labels.note(note));
                        }
                        if entry.clicked() {
                            selected.clone_from(&scanner.id);
                        }
                    }
                })
                .response
                .labelled_by(caption.id);
            if ui.add_enabled(!self.loading, egui::Button::new(labels.refresh)).on_hover_text(labels.refresh_hint).clicked() {
                response.refresh = self.refresh(discover);
            }
            if self.checking() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(labels.checking);
                });
            } else if self.details_error().is_some() && ui.button(labels.try_again).clicked() {
                response.retry = self.retry(load_choices);
            }
        });
    }

    /// Draws the source, colour, resolution, page and length rows for `choices`, editing the settings in place.
    fn controls(&mut self, ui: &mut egui::Ui, labels: &Labels<'_>, choices: &Choices) {
        let warnings = self.warnings.clone();
        let settings = &mut self.settings;
        let width = ui.available_width().clamp(32.0, 180.0);
        let feeder_reason = labels.reason(labels.reasons.feeder);
        egui::Grid::new("controls").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            let caption = ui.label(labels.source);
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    for source in [Source::Auto, Source::Glass, Source::Feeder] {
                        let offered = choices.offers(source);
                        let shown = ui.add_enabled_ui(offered, |ui| ui.selectable_value(&mut settings.source, source, labels.source(source))).inner;
                        if !offered {
                            let reason = if source == Source::Glass { labels.reason(labels.reasons.glass) } else { feeder_reason };
                            shown.on_disabled_hover_text(reason);
                        }
                    }
                });
                let note = if choices.feeder.is_unsupported() {
                    feeder_reason
                } else if choices.glass.is_unsupported() {
                    labels.reason(labels.reasons.glass)
                } else if !choices.feeder.is_known() || !choices.glass.is_known() {
                    labels.unconfirmed_note
                } else {
                    ""
                };
                quiet(ui, note);
            })
            .response
            .labelled_by(caption.id);
            ui.end_row();

            let caption = ui.label(labels.resolution);
            ui.vertical(|ui| {
                ui.add_enabled_ui(choices.dpi.is_enabled(), |ui| {
                    egui::ComboBox::from_id_salt("dpi")
                        .width(width)
                        .selected_text(format!("{} {}", settings.dpi, labels.dpi))
                        .show_ui(ui, |ui| {
                            for value in choices.dpi_options() {
                                if ui.selectable_value(&mut settings.dpi, value, format!("{value} {}", labels.dpi)).clicked() {
                                    settings.preset = None;
                                }
                            }
                        })
                        .response
                        .labelled_by(caption.id);
                });
                quiet(ui, labels.note_for(&choices.dpi, labels.reasons.dpi));
            });
            ui.end_row();

            let caption = ui.label(labels.color);
            ui.vertical(|ui| {
                ui.add_enabled_ui(choices.colors.is_enabled(), |ui| {
                    egui::ComboBox::from_id_salt("color")
                        .width(width)
                        .selected_text(labels.color(settings.color))
                        .show_ui(ui, |ui| {
                            for color in choices.color_options() {
                                if ui.selectable_value(&mut settings.color, color, labels.color(color)).clicked() {
                                    settings.preset = None;
                                }
                            }
                        })
                        .response
                        .labelled_by(caption.id);
                });
                quiet(ui, labels.note_for(&choices.colors, labels.reasons.colors));
            });
            ui.end_row();

            // Hidden only when the user chose the glass; with no feeder the rows stay, disabled.
            let in_play = choices.feeder_in_play(settings.source);
            if in_play || choices.feeder.is_unsupported() {
                let caption = ui.label(labels.pages);
                ui.vertical(|ui| {
                    ui.add_enabled_ui(in_play, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            // Shown in every state like the other options: disabled (with the reason below) when the
                            // device said it cannot scan until the feeder is empty.
                            let mut all = settings.pages == Pages::All;
                            let checkbox = egui::Checkbox::new(&mut all, labels.all_pages);
                            if ui.add_enabled(choices.all_pages.is_enabled(), checkbox).changed() {
                                settings.pages = if all { Pages::All } else { Pages::Count(1) };
                            }
                            if let Pages::Count(count) = &mut settings.pages {
                                ui.add_enabled(choices.max_pages.is_enabled(), egui::DragValue::new(count).range(1..=choices.page_limit()))
                                    .labelled_by(caption.id);
                            }
                        });
                    });
                    let note = if choices.feeder.is_unsupported() {
                        feeder_reason
                    } else if choices.max_pages.is_unsupported() || choices.all_pages.is_unsupported() {
                        labels.reason(labels.reasons.pages)
                    } else if !choices.max_pages.is_known() || !choices.all_pages.is_known() {
                        labels.unconfirmed_note
                    } else {
                        ""
                    };
                    quiet(ui, note);
                });
                ui.end_row();

                let caption = ui.label(labels.duplex);
                ui.vertical(|ui| {
                    ui.add_enabled(in_play && choices.duplex.is_enabled(), egui::Checkbox::new(&mut settings.duplex, labels.duplex_both_sides))
                        .labelled_by(caption.id);
                    let note = if choices.feeder.is_unsupported() { feeder_reason } else { labels.note_for(&choices.duplex, labels.reasons.duplex) };
                    quiet(ui, note);
                });
                ui.end_row();

                // The page-length choice; it belongs to the feeder, so it follows the same rule as the rows above.
                if in_play {
                    let caption = ui.label(labels.length.heading);
                    let value = choices.length_or_default(settings.page_length, settings.page_size_chosen);
                    settings.page_length = value;
                    ui.vertical(|ui| {
                        egui::ComboBox::from_id_salt("page-length")
                            .width(width.max(220.0).min(ui.available_width().max(32.0)))
                            .selected_text(labels.length.option(value))
                            .show_ui(ui, |ui| {
                                for choice in choices.length_choices(settings.page_size_chosen) {
                                    let shown = ui
                                        .add_enabled_ui(choice.availability.is_enabled(), |ui| {
                                            ui.selectable_value(&mut settings.page_length, choice.value, labels.length.option(choice.value))
                                        })
                                        .inner;
                                    if let LengthAvailability::Unavailable(reason) = choice.availability {
                                        shown.on_disabled_hover_text(labels.length.reason(reason));
                                    }
                                }
                            })
                            .response
                            .labelled_by(caption.id);
                        let availability = choices.length_availability(settings.page_length, settings.page_size_chosen);
                        quiet(ui, labels.length.note_for(settings.page_length, availability, labels.unconfirmed_note));
                    });
                    ui.end_row();
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for preset in Preset::ALL {
                let offered = preset.offered(choices);
                let button = ui.add_enabled_ui(offered, |ui| ui.selectable_label(settings.preset == Some(preset), labels.preset(preset))).inner;
                if !offered {
                    button.on_disabled_hover_text(labels.reason(labels.reasons.colors));
                } else if button.clicked() {
                    settings.apply_preset(preset, choices);
                }
            }
        });
        // Status only warns: nothing above depends on it.
        for warning in warnings {
            ui.colored_label(ui.visuals().warn_fg_color, labels.warning(warning));
        }
    }
}

/// A short, low-key line under a control.
fn quiet(ui: &mut egui::Ui, text: &str) {
    if !text.is_empty() {
        ui.label(egui::RichText::new(text).small().weak());
    }
}
