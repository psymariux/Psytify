//! Sezioni della galleria: una funzione per famiglia di componenti.

use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    Styled, Window, div,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{LINE_STRONG, TEXT_PRIMARY, TEXT_TERTIARY};
use psytify_design::tokens::space::{SPACE_3, SPACE_5, SPACE_6, SPACE_7};
use psytify_design::tokens::type_::{TYPE_HEADING, TYPE_LABEL, TYPE_META};

use super::Gallery;
use crate::components::badge::{Badge, BadgeKind};
use crate::components::card::{Card, Panel};
use crate::components::indicators::{
    DownloadIndicator, DownloadState, DownloadSteps, Equalizer, LoadingIndicator, PlayState, Skeleton,
};
use crate::components::list_row::{ListRow, RowArt};
use crate::components::menu::{MenuItem, MenuPanel};
use crate::components::navigation::{NavigationItem, Tab, Tabs};
use crate::components::overlay::{EmptyState, Toast};
use crate::components::page_header::{HeaderArt, PageHeader};
use crate::components::player_control::{PlayerControl, Repeat, Transport};
use crate::components::progress::ProgressBar;
use crate::components::section::{PageTitle, note, section_heading};
use crate::components::select::Select;
use crate::components::separator::Separator;
use crate::components::setting_row::{SettingRow, setting_gap};
use crate::components::slider::SliderBank;
use crate::components::tile::{AlbumTile, Cover};
use crate::components::toggle::Toggle;
use crate::components::tooltip::TooltipBox;
use crate::components::track_row::{TrackRow, sample};
use crate::components::{Button, IconButton, Preview};
use crate::pixel::{PixelSprite, SpriteStore};
use crate::theme::{Ap, TypeStyle, cased, col, dp};

/// Adatta una callback «con valore» dei componenti (`Fn(T, &mut Window, &mut App)`) a un metodo
/// della galleria, tenendo solo un riferimento debole alla vista.
fn value_cb<T: 'static>(
    cx: &Context<Gallery>,
    f: impl Fn(&mut Gallery, T, &mut Context<Gallery>) + 'static,
) -> impl Fn(T, &mut Window, &mut App) + 'static {
    let this = cx.weak_entity();
    move |value, _window, app| {
        // Se la vista è già chiusa non c'è nulla da aggiornare.
        let _ = this.update(app, |gallery, cx| f(gallery, value, cx));
    }
}

pub fn section(title: &str, content: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(dp(SPACE_5))
        .pb(dp(SPACE_7))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(dp(SPACE_3))
                .child(
                    div()
                        .type_role(TYPE_HEADING)
                        .text_color(col(TEXT_PRIMARY))
                        .child(SharedString::from(title.to_owned())),
                )
                .child(crate::components::separator::Separator::horizontal()),
        )
        .child(content)
        .into_any_element()
}

/// Una cella: componente con la sua etichetta (lo stato) sotto.
fn cell(label: &str, content: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(dp(SPACE_3))
        .items_start()
        .child(content)
        .child(div().type_role(TYPE_LABEL).text_color(col(TEXT_TERTIARY)).child(cased(label, TYPE_LABEL)))
        .into_any_element()
}

fn row(cells: Vec<AnyElement>) -> AnyElement {
    div().flex().flex_wrap().items_start().gap(dp(SPACE_6)).children(cells).into_any_element()
}

const STATES: [(&str, Preview); 4] = [
    ("rest", Preview::Rest),
    ("hover", Preview::Hover),
    ("pressed", Preview::Pressed),
    ("focused", Preview::Focused),
];

pub fn body(g: &mut Gallery, window: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let ap = Ap::of(window);
    let scale = window.scale_factor();
    let (entries, bytes) = cx.global::<SpriteStore>().cache_stats();
    let sections: Vec<AnyElement> = vec![
        section(
            &format!(
                "Sprite · scala {scale:.2} → 1 AP = {} px fisici · cache {entries} voci, {} KiB",
                ap.device,
                bytes / 1024
            ),
            sprites(),
        ),
        section("Button · primary, secondary, ghost, destructive", buttons()),
        section("IconButton", icon_buttons(g, cx)),
        section("Badge, Separator, Tooltip", badges()),
        section("Card e Panel", cards()),
        section("Menu e menu contestuale (clic destro nel riquadro)", menus(cx)),
        section("ProgressBar (seek) e Slider (volume)", progress(g, cx)),
        section("SearchField", search(g, cx)),
        section("Campi dei moduli · 40 dp, in chiaro e mascherato (password, codice)", form_fields(g)),
        section("PersonRow, account nella barra del titolo e pulsanti dei provider", people(cx)),
        section("Toggle", toggles(g, cx)),
        section("Impostazioni · SettingRow, Select e SliderBank (bande dell'equalizzatore)", settings(g, cx)),
        section("NavigationItem e Tabs", navigation(g, cx)),
        section("PlayerControl", player(g, cx)),
        section("TrackRow", track_rows()),
        section("Titolo di pagina, intestazione di sezione, riga di lista e nota", page_parts()),
        section("AlbumTile e ArtistTile", tiles()),
        section("Sidebar v2 · library rows, rail covers and the Jam card", sidebar_parts(window, cx)),
        section("Shelf (frecce, rotella in orizzontale, «Mostra tutto») e Shortcut", shelves(g, cx)),
        section("Equalizer, LoadingIndicator, DownloadIndicator", indicators()),
        section("Toast e Dialog", overlays(cx)),
        section("EmptyState e Nastro", empty_states()),
        section("Now playing v2 · side panel, stage, dissolve and like", super::now_playing::now_playing()),
    ];
    div().flex().flex_col().children(sections).into_any_element()
}

fn sprites() -> AnyElement {
    let Ok(sheet) = psytify_design::sprite::sheet() else {
        return div().child("sprite non disponibili").into_any_element();
    };
    let items = sheet.iter().map(|s| {
        let tint = (s.kind() == psytify_design::sprite::SpriteKind::Mask)
            .then_some(psytify_design::tokens::color::ICON_ACTIVE);
        let mut sprite = PixelSprite::new(
            psytify_design::sprite::ALL_NAMES
                .iter()
                .find(|n| **n == s.name())
                .copied()
                .unwrap_or(sprite::MARK),
        );
        if let Some(tint) = tint {
            sprite = sprite.tint(tint);
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(dp(SPACE_3))
            .w(dp(96.0))
            .child(div().h(dp(104.0)).flex().items_center().child(sprite))
            .child(
                div()
                    .type_role(TYPE_META)
                    .text_color(col(TEXT_TERTIARY))
                    .truncate()
                    .child(SharedString::from(s.name().to_owned())),
            )
    });
    div().flex().flex_wrap().gap(dp(SPACE_5)).children(items).into_any_element()
}

fn buttons() -> AnyElement {
    type Apply = fn(Button) -> Button;
    let kinds: [(&str, Apply); 4] = [
        ("primary", Button::primary),
        ("secondary", Button::secondary),
        ("ghost", Button::ghost),
        ("destructive", Button::destructive),
    ];
    let mut rows = Vec::new();
    for (kind, apply) in kinds {
        let mut cells = Vec::new();
        for (state, preview) in STATES {
            cells.push(cell(
                state,
                apply(
                    Button::new(format!("btn-{kind}-{state}"), "Riproduci")
                        .icon(sprite::ICON_PLAY)
                        .preview(preview),
                ),
            ));
        }
        cells.push(cell(
            "disabled",
            apply(Button::new(format!("btn-{kind}-dis"), "Riproduci").disabled(true)),
        ));
        cells
            .push(cell("loading", apply(Button::new(format!("btn-{kind}-load"), "Riproduci").loading(true))));
        rows.push(row(cells));
    }
    rows.push(row(vec![
        cell("selected (toggle)", Button::new("btn-sel", "Segui già").secondary().selected(true)),
        cell("large 40", Button::new("btn-large", "Riproduci").primary().large().icon(sprite::ICON_PLAY)),
        cell("live", Button::new("btn-live", "Clicca me").primary()),
    ]));
    div().flex().flex_col().gap(dp(SPACE_5)).children(rows).into_any_element()
}

fn icon_buttons(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let mut cells = Vec::new();
    for (state, preview) in STATES {
        cells.push(cell(state, IconButton::new(format!("ib-{state}"), sprite::ICON_QUEUE).preview(preview)));
    }
    cells.push(cell("disabled", IconButton::new("ib-dis", sprite::ICON_QUEUE).disabled(true)));
    cells.push(cell("selected (lime)", IconButton::new("ib-sel", sprite::ICON_SHUFFLE).selected(true)));
    cells.push(cell(
        "selected (cuore, bianco)",
        IconButton::new("ib-heart", sprite::ICON_HEART)
            .icon_on(sprite::ICON_HEART_FILL)
            .on_style(crate::components::icon_button::OnStyle::Active)
            .selected(true),
    ));
    cells.push(cell("loading", IconButton::new("ib-load", sprite::ICON_QUEUE).loading(true)));
    cells.push(cell(
        "live + tooltip",
        IconButton::new("ib-live", sprite::ICON_SETTINGS)
            .tooltip("Impostazioni", Some("Ctrl ,"))
            .on_click(cx.listener(|this, _, _, cx| this.show_toast("Impostazioni", cx))),
    ));
    let _ = g;
    row(cells)
}

fn badges() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(dp(SPACE_5))
        .child(row(vec![
            cell("neutral", Badge::new("MP3 320", BadgeKind::Neutral)),
            cell("quality", Badge::new("FLAC 24/96", BadgeKind::Quality)),
            cell("count", Badge::new("12", BadgeKind::Count)),
            cell("live", Badge::new("Live", BadgeKind::Live)),
            cell("error", Badge::new("Errore", BadgeKind::Error)),
            cell("disabled", Badge::new("CC BY", BadgeKind::Neutral).disabled(true)),
        ]))
        .child(row(vec![
            cell("tooltip", TooltipBox::new("Riproduci").shortcut("Spazio")),
            cell(
                "separator (sottile, forte)",
                div()
                    .w(dp(160.0))
                    .flex()
                    .flex_col()
                    .gap(dp(SPACE_3))
                    .child(Separator::horizontal())
                    .child(Separator::horizontal().strong()),
            ),
        ]))
        .into_any_element()
}

fn cards() -> AnyElement {
    let card = |id: &'static str, label: &'static str, f: fn(Card) -> Card| {
        cell(
            label,
            f(Card::new().interactive(id))
                .child(div().w(dp(160.0)).type_role(TYPE_META).child("Sta ascoltando")),
        )
    };
    row(vec![
        cell(
            "rest (statica)",
            Card::new().child(div().w(dp(160.0)).type_role(TYPE_META).child("Sta ascoltando")),
        ),
        card("card-hover", "hover", |c| c.preview(Preview::Hover)),
        card("card-pressed", "pressed", |c| c.preview(Preview::Pressed)),
        card("card-focus", "focused", |c| c.preview(Preview::Focused)),
        card("card-sel", "selected", |c| c.selected(true)),
        card("card-dis", "disabled", |c| c.disabled(true)),
        cell(
            "panel",
            div().w(dp(240.0)).h(dp(120.0)).child(
                Panel::new()
                    .title("In riproduzione")
                    .action(IconButton::new("panel-x", sprite::ICON_CLOSE).tooltip("Chiudi", None))
                    .child(
                        div()
                            .p(dp(SPACE_5))
                            .type_role(TYPE_META)
                            .text_color(col(TEXT_TERTIARY))
                            .child("Regione del chrome"),
                    ),
            ),
        ),
    ])
}

fn menu_items() -> MenuPanel {
    MenuPanel::new()
        .item(MenuItem::new("m1", "Riproduci").icon(sprite::ICON_PLAY).shortcut("Invio"))
        .item(MenuItem::new("m2", "Aggiungi alla coda").icon(sprite::ICON_QUEUE).shortcut("Q"))
        .item(MenuItem::new("m3", "Vai all'album").icon(sprite::ICON_LIBRARY).submenu(true))
        .separator()
        .item(MenuItem::new("m4", "Mi piace").checked(true))
        .item(MenuItem::new("m5", "Scarica").icon(sprite::ICON_DOWNLOAD).disabled(true))
        .item(MenuItem::new("m6", "Rimuovi").icon(sprite::ICON_TRASH).destructive(true))
}

pub fn context_menu(cx: &mut App) -> impl IntoElement {
    let _ = cx;
    menu_items()
}

fn menus(cx: &mut Context<Gallery>) -> AnyElement {
    let target = div()
        .id("menu-target")
        .w(dp(240.0))
        .h(dp(120.0))
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(col(LINE_STRONG))
        .type_role(TYPE_META)
        .text_color(col(TEXT_TERTIARY))
        .child("clic destro qui")
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                this.menu = Some(event.position);
                cx.notify();
            }),
        );
    row(vec![
        cell(
            "pannello (rest, hover, disabilitato, distruttivo, spunta)",
            div().pr(dp(SPACE_3)).pb(dp(SPACE_3)).child(menu_items()),
        ),
        cell(
            "voce hover / premuta",
            div().w(dp(240.0)).child(
                MenuPanel::new()
                    .item(MenuItem::new("mh", "Hover").icon(sprite::ICON_PLAY).preview(Preview::Hover))
                    .item(MenuItem::new("mp", "Premuta").icon(sprite::ICON_PLAY).preview(Preview::Pressed)),
            ),
        ),
        cell("menu contestuale", target),
    ])
}

fn progress(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let fmt = |frac: f32| {
        let s = (frac * super::DEMO_SECONDS) as u32;
        format!("{}:{:02}", s / 60, s % 60)
    };
    let seek = |id: &'static str, v: f32, p: Preview| {
        div().w(dp(280.0)).child(ProgressBar::new(id, v).buffered(0.5).preview(p).label(fmt))
    };
    let pos = g.position;
    let vol = g.volume;
    row(vec![
        cell("rest", seek("pb-rest", 0.25, Preview::Rest)),
        cell("hover (pomello + anteprima)", seek("pb-hover", 0.25, Preview::Hover)),
        cell("pressed / dragging", seek("pb-press", 0.25, Preview::Pressed)),
        cell("focused", seek("pb-focus", 0.25, Preview::Focused)),
        cell("disabled", div().w(dp(280.0)).child(ProgressBar::new("pb-dis", 0.25).disabled(true))),
        cell(
            "playing (live, trascinabile, ←/→)",
            div().w(dp(360.0)).child(
                ProgressBar::new("pb-live", pos).buffered((pos + 0.2).min(1.0)).label(fmt).on_change(
                    value_cb(cx, |this, v, cx| {
                        this.position = v;
                        cx.notify();
                    }),
                ),
            ),
        ),
        cell(
            "slider volume (live, rotella ±5%)",
            ProgressBar::slider("vol-live", vol)
                .label(|f| format!("{}%", (f * 100.0).round() as u32))
                .on_change(value_cb(cx, |this, v, cx| {
                    this.volume = v;
                    cx.notify();
                })),
        ),
        cell("slider disabled", ProgressBar::slider("vol-dis", 0.4).disabled(true)),
    ])
}

fn search(g: &Gallery, _cx: &mut Context<Gallery>) -> AnyElement {
    let field =
        |e: &gpui::Entity<crate::components::search_field::SearchField>| div().w(dp(360.0)).child(e.clone());
    row(vec![
        cell(
            "live (anche nella barra del titolo): hover, focus bianco, Esc cancella, X cancella",
            field(&g.search_live),
        ),
        cell("con testo", field(&g.search_text)),
        cell("loading (loader al posto della lente)", field(&g.search_loading)),
        cell("disabled", field(&g.search_disabled)),
    ])
}

fn form_fields(g: &Gallery) -> AnyElement {
    let field = |e: &gpui::Entity<crate::components::search_field::SearchField>| {
        div().w(dp(psytify_design::tokens::size::SIGN_IN_W)).child(e.clone())
    };
    row(vec![
        cell("in chiaro (email, nome utente)", field(&g.form_email)),
        cell("mascherato: un pallino per carattere, niente appunti", field(&g.form_password)),
        cell("mascherato vuoto (il codice)", field(&g.form_code)),
    ])
}

fn people(_cx: &mut Context<Gallery>) -> AnyElement {
    use crate::components::person_row::{PersonRow, Status};
    use crate::titlebar::{AccountChip, account};
    let person = |id: &'static str, name: &'static str, status: Status| {
        div().w(dp(560.0)).child(PersonRow::new(id, name, status))
    };
    let chips = [
        ("fuori", AccountChip::SignedOut),
        ("in corso", AccountChip::Working),
        ("dentro", AccountChip::SignedIn { avatar: None, name: "Psy".into() }),
    ];
    let providers = [
        ("Google", psytify_design::brand::GOOGLE),
        ("GitHub", psytify_design::brand::GITHUB),
        ("Discord", psytify_design::brand::DISCORD),
    ];
    div()
        .flex()
        .flex_col()
        .gap(dp(SPACE_5))
        .child(row(vec![
            cell(
                "sta ascoltando (punto lime)",
                person(
                    "pr-live",
                    "Luca Bianchi",
                    Status::Live("Sta ascoltando Neon Harbour · Faro Nord".into()),
                ),
            ),
            cell(
                "in pausa",
                person("pr-paused", "Sara Verdi", Status::Plain("In pausa: Vetro · Lumen Quartet".into())),
            ),
        ]))
        .child(row(vec![
            cell(
                "richiesta: Accetta e Rifiuta",
                div().w(dp(560.0)).child(
                    PersonRow::new("pr-request", "Giulia Rossi", Status::Plain("@giulia".into()))
                        .action(Button::new("pr-accept", "Accetta").secondary())
                        .action(Button::new("pr-reject", "Rifiuta").ghost()),
                ),
            ),
            cell(
                "hover e focus",
                div().w(dp(560.0)).child(
                    PersonRow::new("pr-hover", "Marco Neri", Status::Plain("@marco".into()))
                        .preview(Preview::Hover),
                ),
            ),
        ]))
        .child(row(chips
            .into_iter()
            .map(|(label, chip)| cell(label, div().children(account(chip, None))))
            .collect()))
        .child(row(providers
            .into_iter()
            .enumerate()
            .map(|(ix, (name, mark))| {
                cell(
                    name,
                    Button::new(("pr-provider", ix), format!("Continua con {name}"))
                        .secondary()
                        .large()
                        .width(psytify_design::tokens::size::SIGN_IN_W)
                        .mark(mark),
                )
            })
            .collect()))
        .into_any_element()
}

fn toggles(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let live =
        |ix: usize, id: &'static str, label: &'static str, desc: &'static str, cx: &mut Context<Gallery>| {
            Toggle::new(id, g.toggles[ix]).label(label).description(desc).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.toggles[ix] = !this.toggles[ix];
                    cx.notify();
                },
            ))
        };
    row(vec![
        cell(
            "off (live)",
            div().w(dp(320.0)).child(live(1, "tg-0", "Crossfade", "Sfuma tra un brano e il successivo", cx)),
        ),
        cell(
            "on (live)",
            div().w(dp(320.0)).child(live(
                0,
                "tg-1",
                "Normalizza il volume",
                "Livello uguale per ogni brano",
                cx,
            )),
        ),
        cell("hover", Toggle::new("tg-h", false).preview(Preview::Hover)),
        cell("pressed", Toggle::new("tg-p", false).preview(Preview::Pressed)),
        cell("focused", Toggle::new("tg-f", true).preview(Preview::Focused)),
        cell("disabled off", Toggle::new("tg-d0", false).disabled(true)),
        cell("disabled on", Toggle::new("tg-d1", true).disabled(true)),
    ])
}

fn settings(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let options: Vec<SharedString> =
        ["Lossless · FLAC o WAV", "Alta · AAC 256 kbps", "Media · AAC 128 kbps", "Bassa · AAC 64 kbps"]
            .into_iter()
            .map(SharedString::from)
            .collect();
    let live =
        Select::new("sel-live", options.clone(), Some(g.choice)).on_select(value_cb(cx, |this, ix, cx| {
            this.choice = ix;
            cx.notify();
        }));
    let toggle = Toggle::new("set-toggle", g.toggles[2]).on_click(cx.listener(|this, _, _, cx| {
        this.toggles[2] = !this.toggles[2];
        cx.notify();
    }));
    let rows = div()
        .w(dp(720.0))
        .flex()
        .flex_col()
        .child(section_heading("Qualità".into(), SharedString::default()))
        .child(
            SettingRow::new("Qualità dei download")
                .description("AAC 256 kbps: alta qualità, circa 8 MB a brano. Dal menu di un brano si sceglie caso per caso.")
                .control(live),
        )
        .child(
            SettingRow::new("Riproduzione automatica")
                .description("Finita la coda, continua da solo con altri brani dello stesso artista.")
                .control(toggle),
        )
        .child(setting_gap())
        .child(section_heading("Informazioni".into(), SharedString::default()))
        .child(SettingRow::new("Server").description("Indirizzo non valido.").error(true).control(Button::new("set-use", "Usa").secondary()));
    let gains = g.bands;
    let bank = SliderBank::new("gal-bands", gains.to_vec())
        .values_text(gains.iter().map(|v| SharedString::from(format!("{:+.0}", (v - 0.5) * 24.0))).collect())
        .names(
            ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"]
                .into_iter()
                .map(SharedString::from)
                .collect(),
        )
        .tick(0.5)
        .step(1.0 / 48.0)
        .preview(2, Preview::Hover)
        .preview(5, Preview::Focused);
    let bank = {
        let this = cx.weak_entity();
        bank.on_change(move |band, value, _, app| {
            let _ = this.update(app, |gallery, cx| {
                if let Some(slot) = gallery.bands.get_mut(band) {
                    *slot = value;
                    cx.notify();
                }
            });
        })
    };
    row(vec![
        cell("righe di impostazione (gruppo, separatore fra gruppi, descrizione d'errore)", rows),
        cell("select rest", Select::new("sel-rest", options.clone(), Some(0)).preview(Preview::Rest)),
        cell("select hover", Select::new("sel-hover", options.clone(), Some(0)).preview(Preview::Hover)),
        cell("select focused", Select::new("sel-focus", options.clone(), Some(0)).preview(Preview::Focused)),
        cell("select disabled", Select::new("sel-dis", options.clone(), Some(0)).disabled(true)),
        cell("select vuoto", Select::new("sel-none", options, None).placeholder("Personalizzato")),
        cell("banco di slider (hover sulla 3ª, focus sulla 6ª, linea dello 0)", bank),
    ])
}

fn navigation(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let items = [
        ("Home", sprite::ICON_HOME, sprite::ICON_HOME_FILL),
        ("Cerca", sprite::ICON_SEARCH, sprite::ICON_SEARCH_FILL),
        ("Libreria", sprite::ICON_LIBRARY, sprite::ICON_LIBRARY_FILL),
    ];
    let nav = items.iter().copied().enumerate().map(|(ix, (label, icon, filled))| {
        NavigationItem::new(("nav", ix), label, icon).icon_selected(filled).selected(g.nav == ix).on_click(
            cx.listener(move |this, _, _, cx| {
                this.nav = ix;
                cx.notify();
            }),
        )
    });
    let tabs = Tabs::new(
        vec![Tab::new("tab-0", "In riproduzione"), Tab::new("tab-1", "Coda"), Tab::new("tab-2", "Testo"), {
            let mut t = Tab::new("tab-3", "Disabilitata");
            t.disabled = true;
            t
        }],
        g.tab,
    )
    .on_select(value_cb(cx, |this: &mut Gallery, ix: usize, cx| {
        this.tab = ix;
        cx.notify();
    }));
    row(vec![
        cell(
            "sidebar (live: clic per selezionare)",
            div().w(dp(240.0)).bg(col(psytify_design::tokens::color::SURFACE_CHROME)).children(nav),
        ),
        cell(
            "stati: hover, pressed, focused, disabled, count",
            div()
                .w(dp(240.0))
                .bg(col(psytify_design::tokens::color::SURFACE_CHROME))
                .child(NavigationItem::new("nv-h", "Hover", sprite::ICON_PLAY).preview(Preview::Hover))
                .child(NavigationItem::new("nv-p", "Premuta", sprite::ICON_PLAY).preview(Preview::Pressed))
                .child(NavigationItem::new("nv-f", "Focus", sprite::ICON_PLAY).preview(Preview::Focused))
                .child(NavigationItem::new("nv-d", "Disabilitata", sprite::ICON_PLAY).disabled(true))
                .child(NavigationItem::new("nv-c", "Novità", sprite::ICON_BELL).count(4)),
        ),
        cell(
            "rail",
            div()
                .w(dp(56.0))
                .bg(col(psytify_design::tokens::color::SURFACE_CHROME))
                .child(
                    NavigationItem::new("rl-0", "Home", sprite::ICON_HOME)
                        .icon_selected(sprite::ICON_HOME_FILL)
                        .selected(true)
                        .rail(true),
                )
                .child(NavigationItem::new("rl-1", "Cerca", sprite::ICON_SEARCH).rail(true)),
        ),
        cell("tabs (live)", div().w(dp(380.0)).child(tabs)),
        cell(
            "tabs: hover, pressed, focused",
            div().w(dp(380.0)).child(
                Tabs::new(
                    vec![Tab::new("tb-0", "Hover"), Tab::new("tb-1", "Premuta"), Tab::new("tb-2", "Focus")],
                    99,
                )
                .preview(0, Preview::Hover)
                .preview(1, Preview::Pressed)
                .preview(2, Preview::Focused),
            ),
        ),
    ])
}

fn player(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let transport = if g.playing { Transport::Playing } else { Transport::Stopped };
    row(vec![
        cell(
            "live (play avvia il conteggio e l'equalizzatore)",
            PlayerControl::new("pc-live")
                .transport(transport)
                .shuffle(g.toggles[0])
                .repeat(Repeat::All)
                .on_play(cx.listener(|this, _, _, cx| {
                    let next = !this.playing;
                    this.set_playing(next, cx);
                })),
        ),
        cell("repeat one, shuffle off", PlayerControl::new("pc-one").repeat(Repeat::One)),
        cell("play hover", PlayerControl::new("pc-hover").play_preview(Preview::Hover)),
        cell("play pressed", PlayerControl::new("pc-press").play_preview(Preview::Pressed)),
        cell("play focused", PlayerControl::new("pc-focus").play_preview(Preview::Focused)),
        cell("loading", PlayerControl::new("pc-load").transport(Transport::Loading)),
        cell("disabled (coda vuota)", PlayerControl::new("pc-dis").disabled(true)),
    ])
}

fn track_rows() -> AnyElement {
    let one = |id: &'static str, label: &'static str, f: fn(TrackRow) -> TrackRow| {
        cell(label, div().w(dp(760.0)).child(f(TrackRow::new(id, sample(1)))))
    };
    div()
        .flex()
        .flex_col()
        .gap(dp(SPACE_3))
        .child(one("tr-rest", "rest", |r| r.preview(Preview::Rest)))
        .child(one("tr-hover", "hover", |r| r.preview(Preview::Hover)))
        .child(one("tr-press", "pressed", |r| r.preview(Preview::Pressed)))
        .child(one("tr-focus", "focused (anello interno)", |r| r.preview(Preview::Focused)))
        .child(one("tr-sel", "selected", |r| r.selected(true)))
        .child(one("tr-play", "playing", |r| r.playing(PlayState::Playing)))
        .child(one("tr-pause", "paused", |r| r.playing(PlayState::Paused)))
        .child(one("tr-buf", "buffering", |r| r.playing(PlayState::Buffering)))
        .child(cell(
            "disabled (non disponibile)",
            div().w(dp(760.0)).child(TrackRow::new("tr-dis", sample(5))),
        ))
        .into_any_element()
}

fn page_parts() -> AnyElement {
    let framed = |content: AnyElement| {
        div().w(dp(760.0)).border_1().border_color(col(LINE_STRONG)).child(content).into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .gap(dp(SPACE_5))
        .child(cell(
            "schermata (tipo pixel) con meta e pulsanti",
            framed(
                PageTitle::screen("Libreria")
                    .meta("4 playlist · 2 album · 3 artisti")
                    .action(Button::new("pt-new", "Nuova playlist").secondary().icon(sprite::ICON_PLUS))
                    .into_any_element(),
            ),
        ))
        .child(cell(
            "PageHeader: album (copertina 224, kicker, TYPE_DISPLAY, artista, dati e badge, azioni)",
            framed(
                PageHeader::new(HeaderArt::Cover(None), "Album · 2024", "Neon Harbour")
                    .link("ph-artist", "Ada Vale", |_, _, _| {})
                    .meta("12 brani · 48 min")
                    .quality("Lossless")
                    .action(Button::new("ph-play", "Riproduci").primary().icon(sprite::ICON_PLAY))
                    .action(IconButton::new("ph-shuffle", sprite::ICON_SHUFFLE).tooltip("Casuale", None))
                    .action(
                        IconButton::new("ph-save", sprite::ICON_HEART)
                            .icon_on(sprite::ICON_HEART_FILL)
                            .selected(true)
                            .tooltip("Togli dalla libreria", None),
                    )
                    .into_any_element(),
            ),
        ))
        .child(cell(
            "PageHeader: artista (foto 160 nel cerchio pixel), «Brani che ti piacciono», caricamento",
            div()
                .flex()
                .flex_col()
                .gap(dp(SPACE_3))
                .child(framed(
                    PageHeader::new(HeaderArt::Round(None), "Artista", "Orbit Choir")
                        .meta("4 album · 6 singoli ed EP · 12.400 fan")
                        .action(Button::new("ph-aplay", "Riproduci").primary().icon(sprite::ICON_PLAY))
                        .action(Button::new("ph-follow", "Segui").secondary().icon(sprite::ICON_USER_PLUS))
                        .action(Button::new("ph-radio", "Radio").secondary().icon(sprite::ICON_RADIO))
                        .into_any_element(),
                ))
                .child(framed(
                    PageHeader::new(
                        HeaderArt::Sprite(sprite::ICON_HEART_FILL),
                        "Playlist",
                        "Brani che ti piacciono",
                    )
                    .meta("300 brani · 17 h 2 min")
                    .into_any_element(),
                ))
                .child(framed(PageHeader::loading(false, 24.0).into_any_element()))
                .into_any_element(),
        ))
        .child(cell(
            "PageHeader stretta (520): titolo lungo su due righe con «…», dati troncati, azioni a capo",
            framed(
                div()
                    .w(dp(520.0))
                    .child(
                        PageHeader::new(
                            HeaderArt::Cover(None),
                            "Album · 2025",
                            "Live at Borderland Music Festival, 2025-09-14, with the Orbit Choir and friends (Deluxe)",
                        )
                        .link("ph-long-artist", "Ada Vale, the Orbit Choir and the Harbour Strings", |_, _, _| {})
                        .meta("18 brani · 1 h 48 min · registrato dal vivo a Borderland")
                        .quality("Lossless")
                        .action(Button::new("ph-long-play", "Riproduci").primary().icon(sprite::ICON_PLAY))
                        .action(Button::new("ph-long-save", "Salva nella libreria").secondary())
                        .action(IconButton::new("ph-long-dl", sprite::ICON_DOWNLOAD).tooltip("Scarica tutto", None)),
                    )
                    .into_any_element(),
            ),
        ))
        .child(cell(
            "intestazione di sezione (48 dp, come una riga)",
            framed(section_heading("Sul dispositivo".into(), "12 brani".into()).into_any_element()),
        ))
        .child(cell(
            "riga di lista: playlist, sprite, scaricato in corso, file perso, in hover",
            framed(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        ListRow::new(
                            "lr-liked",
                            RowArt::Sprite(sprite::ICON_HEART_FILL),
                            "Brani che ti piacciono",
                        )
                        .detail("Playlist · 300 brani"),
                    )
                    .child(
                        ListRow::new("lr-list", RowArt::Cover(None), "Sera d'estate")
                            .detail("Playlist · 13 brani")
                            .hover_action(
                                IconButton::new("lr-edit", sprite::ICON_EDIT).tooltip("Rinomina", None),
                            )
                            .preview(Preview::Hover),
                    )
                    .child(
                        ListRow::new("lr-busy", RowArt::Cover(None), "Night Ferry")
                            .detail("Ada Vale · 42%")
                            .steps(0.42)
                            .action(
                                IconButton::new("lr-cancel", sprite::ICON_CLOSE).tooltip("Annulla", None),
                            ),
                    )
                    .child(
                        ListRow::new("lr-lost", RowArt::Cover(None), "Copper Skies")
                            .index(4)
                            .detail("The Quiet Engines · FLAC · 32,1 MB · File mancante")
                            .detail_error(true),
                    )
                    .child(
                        ListRow::new("lr-play", RowArt::Round(None), "Static Bloom")
                            .detail("Mio Kanda · FLAC · 28,0 MB")
                            .playing(PlayState::Playing),
                    )
                    .into_any_element(),
            ),
        ))
        .child(cell(
            "nota (48 dp)",
            framed(note(LoadingIndicator::new().caption("Controllo dei file…")).into_any_element()),
        ))
        .into_any_element()
}

/// The v2 sidebar parts in their states: library rows (rest, hover, selected), the rail, the Jam card.
fn sidebar_parts(window: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    use crate::app::sidebar::{JamCard, LibraryItem, jam_card_element};
    use psytify_design::tokens::color::SURFACE_CHROME;
    use psytify_design::tokens::size::{SIDEBAR_DEFAULT_W, SIDEBAR_RAIL_W};
    let item = |id: &'static str, art: RowArt, title: &'static str, meta: &'static str| {
        LibraryItem::new(id, art, title.into(), meta.into())
    };
    let expanded = div()
        .w(dp(SIDEBAR_DEFAULT_W))
        .flex()
        .flex_col()
        .bg(col(SURFACE_CHROME))
        .child(item(
            "sb-liked",
            RowArt::Sprite(sprite::ICON_HEART_FILL),
            "Brani che ti piacciono",
            "Playlist · 24 brani",
        ))
        .child(item("sb-hover", RowArt::Cover(None), "Notte", "Playlist · 12 brani").preview(Preview::Hover))
        .child(item("sb-sel", RowArt::Cover(None), "Discovery", "Album · Daft Punk").selected(true))
        .child(item("sb-artist", RowArt::Round(None), "Orbit Choir", "Artista"));
    let rail = div()
        .w(dp(SIDEBAR_RAIL_W))
        .flex()
        .flex_col()
        .bg(col(SURFACE_CHROME))
        .child(
            item("sb-r0", RowArt::Sprite(sprite::ICON_HEART_FILL), "Brani che ti piacciono", "Playlist")
                .rail(true),
        )
        .child(item("sb-r1", RowArt::Cover(None), "Notte", "Playlist").rail(true).selected(true))
        .child(item("sb-r2", RowArt::Round(None), "Orbit Choir", "Artista").rail(true));
    let card = JamCard {
        members: vec!["ada".into(), "bruno".into(), "cleo".into(), "dario".into(), "eva".into()],
        meta: "5 persone".into(),
    };
    let jam = div().w(dp(SIDEBAR_DEFAULT_W)).p(dp(SPACE_3)).bg(col(SURFACE_CHROME)).child(jam_card_element(
        &card,
        std::rc::Rc::new(|_, _, _| {}),
        window,
        cx,
    ));
    row(vec![
        cell("expanded: rest, hover, selected, artist", expanded),
        cell("rail (72 dp)", rail),
        cell("Jam card (56 dp, LIVE)", jam),
    ])
}

fn tiles() -> AnyElement {
    row(vec![
        cell("rest", AlbumTile::new("at-rest", "Neon Harbour", "2024 · Album").side(160.0)),
        cell(
            "hover (Play)",
            AlbumTile::new("at-hover", "Neon Harbour", "2024 · Album")
                .side(160.0)
                .preview(Preview::Hover)
                .on_play(|_, _, _| {}),
        ),
        cell(
            "ArtistTile (cerchio pixel), hover",
            AlbumTile::new("at-artist", "Orbit Choir", "Artista").side(160.0).round().preview(Preview::Hover),
        ),
        cell(
            "pressed",
            AlbumTile::new("at-press", "Neon Harbour", "2024 · Album").side(160.0).preview(Preview::Pressed),
        ),
        cell(
            "focused",
            AlbumTile::new("at-focus", "Neon Harbour", "2024 · Album").side(160.0).preview(Preview::Focused),
        ),
        cell("selected", AlbumTile::new("at-sel", "Neon Harbour", "2024 · Album").side(160.0).selected(true)),
        cell(
            "playing",
            AlbumTile::new("at-play", "Neon Harbour", "Ada Vale")
                .side(160.0)
                .playing(PlayState::Playing)
                .on_play(|_, _, _| {}),
        ),
        cell("disabled", AlbumTile::new("at-dis", "Neon Harbour", "2024 · Album").side(160.0).disabled(true)),
        cell(
            "cover 32 / 48 / 96 (segnaposto)",
            div()
                .flex()
                .items_end()
                .gap(dp(SPACE_3))
                .child(Cover::new(32.0))
                .child(Cover::new(48.0))
                .child(Cover::new(96.0)),
        ),
        cell(
            "cerchio pixel 32 / 48 / 96 / 160",
            div()
                .flex()
                .items_end()
                .gap(dp(SPACE_3))
                .child(Cover::new(32.0).circle())
                .child(Cover::new(48.0).circle())
                .child(Cover::new(96.0).circle())
                .child(Cover::new(160.0).circle()),
        ),
    ])
}

/// Copertine di prova dello scaffale (12: tre pagine da quattro nella cornice).
const SHELF_DEMO: [&str; 12] = [
    "Neon Harbour",
    "Paper Lanterns",
    "Static Bloom",
    "Low Tide Radio",
    "Glass Orchard",
    "Night Ferry",
    "Copper Skies",
    "Slow Machines",
    "Orbit Choir",
    "Mio Kanda",
    "Sol Ferro",
    "Ada Vale",
];

fn shelves(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    use crate::components::shelf::{Shelf, ShelfPage};
    use crate::components::shortcut::Shortcut;
    use psytify_design::tokens::size::GRID_GAP;
    // La cornice tiene quattro copertine da 160: le frecce spostano di una pagina, la rotella di una.
    let page = ShelfPage::new(g.shelf_start, 4, SHELF_DEMO.len());
    let tiles = page.range().map(|ix| {
        let mut tile = AlbumTile::new(("shelf-demo", ix), SHELF_DEMO[ix], "Album · Ada Vale").side(160.0);
        if ix < 2 {
            tile = tile.badge("Nuovo");
        }
        if ix == 2 {
            tile = tile.playing(PlayState::Playing).on_play(|_, _, _| {});
        }
        tile.into_any_element()
    });
    let shelf = Shelf::new("shelf-demo", "Novità dai tuoi artisti", page)
        .meta(format!("{}–{} di {}", page.range().start + 1, page.range().end, SHELF_DEMO.len()))
        .tiles(tiles)
        .on_page(value_cb(cx, |g, start, cx| {
            g.shelf_start = start;
            cx.notify();
        }))
        .on_wheel(value_cb(cx, |g, dx: f32, cx| {
            let page = ShelfPage::new(g.shelf_start, 4, SHELF_DEMO.len());
            let steps = g.shelf_wheel.step(dx, 160.0 + GRID_GAP);
            if steps != 0 {
                g.shelf_start = page.moved(steps);
                cx.notify();
            }
        }))
        .on_show_all("Mostra tutto", |_, _, _| {});
    let framed = div()
        .w(dp(2.0 * SPACE_6 + 4.0 * 160.0 + 3.0 * GRID_GAP))
        .border_1()
        .border_color(col(LINE_STRONG))
        .child(shelf);
    let shortcuts = div()
        .w(dp(560.0))
        .grid()
        .grid_cols(2)
        .gap(dp(SPACE_3))
        .child(Shortcut::new("sc-liked", RowArt::Sprite(sprite::ICON_HEART_FILL), "Brani che ti piacciono"))
        .child(Shortcut::new("sc-list", RowArt::Cover(None), "Sera d'estate").preview(Preview::Hover))
        .child(Shortcut::new("sc-play", RowArt::Cover(None), "Neon Harbour 1").playing(PlayState::Playing))
        .child(Shortcut::new("sc-focus", RowArt::Cover(None), "Concentrazione").preview(Preview::Focused));
    let mix = AlbumTile::new("mix-demo", "Mix di Ada Vale", "Ada Vale, Sol Ferro e altri")
        .side(160.0)
        .mosaic(Vec::new())
        .preview(Preview::Hover)
        .on_play(|_, _, _| {});
    div()
        .flex()
        .flex_col()
        .gap(dp(SPACE_5))
        .child(cell("Shelf: una riga di copertine intere, frecce e «Mostra tutto»", framed))
        .child(row(vec![
            cell("Shortcut: riposo, hover, in riproduzione, focus", shortcuts),
            cell("mix (mosaico 2 x 2; senza copertine, il segnaposto)", mix),
        ]))
        .into_any_element()
}

fn indicators() -> AnyElement {
    row(vec![
        cell("eq playing", Equalizer::new(PlayState::Playing)),
        cell("eq paused", Equalizer::new(PlayState::Paused)),
        cell("buffering", Equalizer::new(PlayState::Buffering)),
        cell("loading + riga", LoadingIndicator::new().caption("Carico la pagina dell'artista…")),
        cell(
            "skeleton",
            div()
                .flex()
                .flex_col()
                .gap(dp(SPACE_3))
                .child(Skeleton::new(160.0, 16.0))
                .child(Skeleton::new(100.0, 12.0)),
        ),
        cell("download: available", DownloadIndicator::new("dl-a", DownloadState::Available)),
        cell("not downloadable", DownloadIndicator::new("dl-n", DownloadState::NotDownloadable)),
        cell("queued", DownloadIndicator::new("dl-q", DownloadState::Queued)),
        cell("downloading", DownloadIndicator::new("dl-p", DownloadState::Downloading(0.4))),
        cell("done", DownloadIndicator::new("dl-d", DownloadState::Done)),
        cell("error", DownloadIndicator::new("dl-e", DownloadState::Error)),
        cell("file lost", DownloadIndicator::new("dl-l", DownloadState::Lost)),
        cell("10 passi (40 %)", DownloadSteps::new(0.4)),
    ])
}

fn overlays(cx: &mut Context<Gallery>) -> AnyElement {
    row(vec![
        cell("toast", Toast::new("Aggiunto alla coda").action(Button::new("toast-undo", "Annulla"))),
        cell("toast errore", Toast::new("Download non riuscito").error(true)),
        cell(
            "live: apri",
            div()
                .flex()
                .gap(dp(SPACE_3))
                .child(
                    Button::new("open-toast", "Mostra toast")
                        .secondary()
                        .on_click(cx.listener(|this, _, _, cx| this.show_toast("Aggiunto alla coda", cx))),
                )
                .child(Button::new("open-dialog", "Apri dialogo").secondary().on_click(cx.listener(
                    |this, _, _, cx| {
                        this.dialog = true;
                        cx.notify();
                    },
                ))),
        ),
    ])
}

fn empty_states() -> AnyElement {
    row(vec![
        cell(
            "empty_crate",
            EmptyState::crate_("Ancora nessun disco", "Cerca un artista e salva un album.")
                .action(Button::new("es-1", "Sfoglia")),
        ),
        cell(
            "nastro_idle",
            EmptyState::new(sprite::NASTRO_IDLE, "Psytify", "Il player che ascolta con te.")
                .action(Button::new("es-2", "Informazioni")),
        ),
        cell(
            "nastro_listen",
            EmptyState::new(sprite::NASTRO_LISTEN, "Nessun testo", "Per questo brano non c'è un testo."),
        ),
        cell(
            "nastro_sleep",
            EmptyState::new(sprite::NASTRO_SLEEP, "Sei offline", "Suonano solo i brani scaricati."),
        ),
    ])
}
