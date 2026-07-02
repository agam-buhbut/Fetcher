use std::collections::VecDeque;

use eframe::egui::{self, Align, Color32, Layout, RichText, ScrollArea};
use egui_extras::{Column, TableBuilder};
use egui_plot::{Line, Plot, PlotPoints};
use taskmgr_core::processes::sort_in_place;
use taskmgr_core::services::{service_action, ServiceOp, ServiceScope};
use taskmgr_core::startup::{set_enabled, AutostartScope};
use taskmgr_core::{
    human_bytes, kill_process, opt_bytes, KillSignal, ProcessRow, SortColumn, SortOrder,
};

use crate::app::{App, Tab, TICK};

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.maybe_tick();

        // Sidebar (Win-style tab strip)
        egui::SidePanel::left("sidebar")
            .min_width(140.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Task Manager");
                ui.separator();
                for (tab, label) in [
                    (Tab::Processes, "Processes"),
                    (Tab::Performance, "Performance"),
                    (Tab::Startup, "Startup"),
                    (Tab::Services, "Services"),
                ] {
                    if ui
                        .selectable_label(self.tab == tab, format!("  {label}"))
                        .clicked()
                    {
                        self.tab = tab;
                        ctx.request_repaint();
                    }
                }
                ui.add_space(8.0);
                ui.separator();
                if let Some((_, msg)) = &self.status {
                    ui.label(RichText::new(msg).color(Color32::LIGHT_YELLOW));
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Processes => draw_processes(ui, self),
            Tab::Performance => draw_performance(ui, self),
            Tab::Startup => draw_startup(ui, self),
            Tab::Services => draw_services(ui, self),
        });

        // Idle when nothing changes — keeps GUI cheap. Schedule the wake-up
        // for when the *current* tick interval expires, not TICK from the end
        // of this frame, so samples don't drift and skip a second.
        ctx.request_repaint_after(TICK.saturating_sub(self.last_tick.elapsed()));
    }
}

/// Right-aligned monospace numeric cell (table cells default left-aligned).
fn num(ui: &mut egui::Ui, text: String) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.monospace(text);
    });
}

/// Left-aligned text cell, truncated with `…` instead of wrapping.
fn text_cell(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) {
    ui.add(egui::Label::new(text).truncate());
}

/// Per-process disk rate cell. A true 0 B/s and a permission-denied
/// `/proc/<pid>/io` read are indistinguishable, so render both as "—"
/// rather than a wall of zeros.
fn rate_str(v: u64) -> String {
    opt_bytes((v > 0).then_some(v))
}

fn draw_performance(ui: &mut egui::Ui, app: &App) {
    ui.heading("Performance");
    ui.separator();

    ScrollArea::vertical().show(ui, |ui| {
        if let Some(c) = &app.snapshot.cpu {
            ui.label(format!(
                "CPU  {:.1}%   ({} cores)",
                c.global_usage,
                c.per_core.len()
            ));
        }
        plot_history(ui, "cpu", &app.cpu_history, Some(100.0));

        if let Some(m) = &app.snapshot.memory {
            ui.label(format!(
                "Memory  {} / {}  ({:.1}%)",
                human_bytes(m.used_bytes),
                human_bytes(m.total_bytes),
                m.used_percent()
            ));
            if m.swap_total_bytes > 0 {
                ui.label(format!(
                    "Swap  {} / {}",
                    human_bytes(m.swap_used_bytes),
                    human_bytes(m.swap_total_bytes)
                ));
            }
        }
        plot_history(ui, "mem", &app.mem_history, Some(100.0));

        ui.label("Disk B/s (R+W)");
        plot_history(ui, "disk", &app.disk_history, None);

        ui.label("Net B/s (RX+TX)");
        plot_history(ui, "net", &app.net_history, None);
    });
}

fn draw_processes(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal(|ui| {
        ui.heading("Processes");
        ui.add_space(20.0);
        ui.label("Filter:");
        ui.text_edit_singleline(&mut app.filter);
    });
    ui.separator();

    let row_h = ui.spacing().interact_size.y;

    // Sort clicks land after the table renders; egui repaints on interaction
    // so the new order is visible the very next frame.
    let mut clicked: Option<SortColumn> = None;
    let mut to_kill: Option<(u32, KillSignal, String)> = None;
    {
        // References only — cloning every row (and all its strings) each
        // frame was the hottest allocation in the GUI.
        let rows: Vec<&ProcessRow> = app
            .snapshot
            .processes
            .as_deref()
            .map(|rs| rs.iter().filter(|r| r.matches(&app.filter)).collect())
            .unwrap_or_default();

        let sort = app.sort;
        let header_label = |c: SortColumn, label: &str| -> RichText {
            let text = if c == sort.column {
                match sort.order {
                    SortOrder::Ascending => format!("{label} ▲"),
                    SortOrder::Descending => format!("{label} ▼"),
                }
            } else {
                label.to_string()
            };
            RichText::new(text).strong()
        };

        // push_id: column-resize state is keyed by Id; keep the two tables
        // (Processes / Services) from sharing widths.
        ui.push_id("proc_table", |ui| {
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::exact(64.0)) // PID
                .column(Column::exact(88.0)) // User
                .column(Column::remainder().at_least(120.0).clip(true)) // Name
                .column(Column::exact(64.0)) // CPU%
                .column(Column::exact(80.0)) // Mem
                .column(Column::exact(80.0)) // DiskR/s
                .column(Column::exact(80.0)) // DiskW/s
                .column(Column::auto().at_least(100.0)) // kill buttons
                .header(row_h + 4.0, |mut h| {
                    for (c, label) in [
                        (Some(SortColumn::Pid), "PID"),
                        (None, "User"),
                        (Some(SortColumn::Name), "Name"),
                        (Some(SortColumn::Cpu), "CPU%"),
                        (Some(SortColumn::Memory), "Mem"),
                        (Some(SortColumn::DiskRead), "DiskR/s"),
                        (Some(SortColumn::DiskWrite), "DiskW/s"),
                        (None, ""),
                    ] {
                        h.col(|ui| match c {
                            // Header doubles as the sort button.
                            Some(c) => {
                                let btn = egui::Button::new(header_label(c, label)).frame(false);
                                if ui.add(btn).clicked() {
                                    clicked = Some(c);
                                }
                            }
                            None => {
                                ui.strong(label);
                            }
                        });
                    }
                })
                .body(|body| {
                    // Virtualized: only visible rows are laid out each frame.
                    body.rows(row_h, rows.len(), |mut trow| {
                        let r = rows[trow.index()];
                        trow.col(|ui| num(ui, r.pid.to_string()));
                        trow.col(|ui| text_cell(ui, r.user.as_str()));
                        trow.col(|ui| text_cell(ui, r.name.as_str()));
                        trow.col(|ui| num(ui, format!("{:.1}", r.cpu_percent)));
                        trow.col(|ui| num(ui, human_bytes(r.memory_bytes)));
                        trow.col(|ui| num(ui, rate_str(r.disk_read_per_sec)));
                        trow.col(|ui| num(ui, rate_str(r.disk_write_per_sec)));
                        trow.col(|ui| {
                            if ui.small_button("End").clicked() {
                                to_kill = Some((r.pid, KillSignal::Term, r.name.clone()));
                            }
                            if ui.small_button("Force").clicked() {
                                to_kill = Some((r.pid, KillSignal::Kill, r.name.clone()));
                            }
                        });
                    });
                });
        });
    }

    if let Some(c) = clicked {
        app.sort.cycle(c);
        if let Some(rows) = &mut app.snapshot.processes {
            sort_in_place(rows, app.sort);
        }
    }
    if let Some((pid, sig, name)) = to_kill {
        match kill_process(pid, sig) {
            Ok(()) => app.set_status(format!("sent {sig:?} to {name} ({pid})")),
            Err(e) => app.set_status(format!("kill {pid} failed: {e}")),
        }
    }
}

fn draw_startup(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal(|ui| {
        ui.heading("Startup");
        if ui.button("Refresh").clicked() {
            app.startup_dirty = true;
        }
    });
    ui.separator();

    let mut to_toggle: Option<(usize, bool)> = None;
    ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("startup_grid")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                for h in ["Enabled", "Name", "Scope", "Exec"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for (i, e) in app.autostart.iter().enumerate() {
                    let mut on = e.enabled;
                    if ui.checkbox(&mut on, "").changed() {
                        to_toggle = Some((i, on));
                    }
                    ui.label(&e.name);
                    ui.label(match e.scope {
                        AutostartScope::User => "user",
                        AutostartScope::System => "system",
                    });
                    ui.label(&e.exec);
                    ui.end_row();
                }
            });
    });
    if let Some((i, on)) = to_toggle {
        if let Some(entry) = app.autostart.get(i).cloned() {
            match set_enabled(&entry, on) {
                Ok(updated) => {
                    app.autostart[i] = updated;
                    app.set_status(format!(
                        "{} {}",
                        if on { "enabled" } else { "disabled" },
                        entry.name
                    ));
                }
                Err(e) => app.set_status(format!("toggle failed: {e}")),
            }
        }
    }
}

fn draw_services(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal(|ui| {
        ui.heading("Services");
        ui.add_space(10.0);
        let user_sel = app.services_scope == ServiceScope::User;
        if ui.selectable_label(user_sel, "User").clicked() {
            app.services_scope = ServiceScope::User;
            app.services_dirty = true;
        }
        if ui.selectable_label(!user_sel, "System").clicked() {
            app.services_scope = ServiceScope::System;
            app.services_dirty = true;
        }
        if ui.button("Refresh").clicked() {
            app.services_dirty = true;
        }
    });
    ui.separator();

    let row_h = ui.spacing().interact_size.y;

    let mut action: Option<(String, ServiceOp)> = None;
    // Virtualized: only the visible slice is laid out each frame (each row
    // carries five buttons, so full layout was the GUI's other hot path).
    ui.push_id("svc_table", |ui| {
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(260.0)) // Name
            .column(Column::exact(70.0)) // Active
            .column(Column::exact(90.0)) // Sub
            .column(Column::remainder().at_least(120.0).clip(true)) // Description
            .column(Column::auto().at_least(280.0)) // Actions
            .header(row_h + 4.0, |mut h| {
                for label in ["Name", "Active", "Sub", "Description", "Actions"] {
                    h.col(|ui| {
                        ui.strong(label);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, app.services.len(), |mut trow| {
                    let u = &app.services[trow.index()];
                    trow.col(|ui| text_cell(ui, u.name.as_str()));
                    trow.col(|ui| {
                        let color = match u.active_state.as_str() {
                            "active" => Color32::LIGHT_GREEN,
                            "failed" => Color32::LIGHT_RED,
                            "inactive" => Color32::GRAY,
                            _ => Color32::YELLOW,
                        };
                        text_cell(ui, RichText::new(&u.active_state).color(color));
                    });
                    trow.col(|ui| text_cell(ui, u.sub_state.as_str()));
                    trow.col(|ui| text_cell(ui, u.description.as_str()));
                    trow.col(|ui| {
                        for (label, op) in [
                            ("Start", ServiceOp::Start),
                            ("Stop", ServiceOp::Stop),
                            ("Restart", ServiceOp::Restart),
                            ("Enable", ServiceOp::Enable),
                            ("Disable", ServiceOp::Disable),
                        ] {
                            if ui.small_button(label).clicked() {
                                action = Some((u.name.clone(), op));
                            }
                        }
                    });
                });
            });
    });
    if let Some((unit, op)) = action {
        match service_action(&unit, op, app.services_scope) {
            Ok(()) => {
                app.set_status(format!("{op:?} {unit}"));
                app.services_dirty = true;
            }
            Err(e) => app.set_status(format!("{op:?} {unit} failed: {e}")),
        }
    }
}

/// Single plot helper — pass `Some(max)` for fixed axes (percent), `None` to
/// auto-fit (bytes).
fn plot_history(ui: &mut egui::Ui, id: &str, data: &VecDeque<f64>, fixed_max: Option<f64>) {
    let pts: PlotPoints = data
        .iter()
        .enumerate()
        .map(|(i, v)| [i as f64, *v])
        .collect();
    let max = fixed_max.unwrap_or_else(|| data.iter().copied().fold(0.0_f64, f64::max).max(1.0));
    Plot::new(id)
        .height(120.0)
        .include_y(0.0)
        .include_y(max)
        .show_axes([false, true])
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .show(ui, |p| {
            p.line(Line::new(pts).fill(0.0));
        });
}
