use crate::{
    cmd,
    data::{
        Album, ArtistTracks, CommonCtx, Ctx, Nav, PlaybackOrigin, PlaybackPayload, PlaylistTracks,
        SavedTracks, SearchResults, State, Track,
    },
    ui::theme,
    widget::LinkExt,
};
use druid::{
    im::Vector,
    kurbo::Line,
    lens::Map,
    piet::StrokeStyle,
    widget::{
        Controller, ControllerHost, CrossAxisAlignment, Flex, Label, List, ListIter, Painter,
    },
    Data, Env, Event, EventCtx, Lens, LensExt, LocalizedString, Menu, MenuItem, MouseButton,
    RenderContext, TextAlignment, Widget, WidgetExt,
};
use std::sync::Arc;

use super::utils;

#[derive(Copy, Clone)]
pub struct TrackDisplay {
    pub number: bool,
    pub title: bool,
    pub artist: bool,
    pub album: bool,
    pub popularity: bool,
}

impl TrackDisplay {
    pub fn empty() -> Self {
        TrackDisplay {
            number: false,
            title: false,
            artist: false,
            album: false,
            popularity: false,
        }
    }
}

pub fn tracklist_widget<T>(mode: TrackDisplay) -> impl Widget<Ctx<CommonCtx, T>>
where
    T: TrackIter + Data,
{
    ControllerHost::new(List::new(move || track_widget(mode)), PlayController)
}

pub trait TrackIter {
    fn origin(&self) -> PlaybackOrigin;
    fn tracks(&self) -> &Vector<Arc<Track>>;
}

impl TrackIter for Album {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Album(self.link())
    }

    fn tracks(&self) -> &Vector<Arc<Track>> {
        &self.tracks
    }
}

impl TrackIter for ArtistTracks {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Artist(self.link())
    }

    fn tracks(&self) -> &Vector<Arc<Track>> {
        &self.tracks
    }
}

impl TrackIter for SearchResults {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Search(self.query.clone())
    }

    fn tracks(&self) -> &Vector<Arc<Track>> {
        &self.tracks
    }
}

impl TrackIter for PlaylistTracks {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Playlist(self.link())
    }

    fn tracks(&self) -> &Vector<Arc<Track>> {
        &self.tracks
    }
}

impl TrackIter for SavedTracks {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Library
    }

    fn tracks(&self) -> &Vector<Arc<Track>> {
        &self.tracks
    }
}

impl<T> ListIter<TrackRow> for Ctx<CommonCtx, T>
where
    T: TrackIter + Data,
{
    fn for_each(&self, mut cb: impl FnMut(&TrackRow, usize)) {
        let origin = self.data.origin();
        let tracks = self.data.tracks();
        ListIter::for_each(tracks, |track, index| {
            let d = TrackRow {
                ctx: self.ctx.to_owned(),
                origin: origin.to_owned(),
                track: track.to_owned(),
                position: index,
            };
            cb(&d, index);
        });
    }

    fn for_each_mut(&mut self, mut cb: impl FnMut(&mut TrackRow, usize)) {
        let origin = self.data.origin();
        let tracks = self.data.tracks();
        ListIter::for_each(tracks, |track, index| {
            let mut d = TrackRow {
                ctx: self.ctx.to_owned(),
                origin: origin.to_owned(),
                track: track.to_owned(),
                position: index,
            };
            cb(&mut d, index);

            // Mutation intentionally ignored.
        });
    }

    fn data_len(&self) -> usize {
        self.data.tracks().len()
    }
}

#[derive(Clone, Data, Lens)]
struct TrackRow {
    ctx: CommonCtx,
    track: Arc<Track>,
    origin: PlaybackOrigin,
    position: usize,
}

impl TrackRow {
    fn is_playing() -> impl Lens<TrackRow, bool> {
        Map::new(
            |tr: &TrackRow| tr.ctx.is_track_playing(&tr.track),
            |_tr: &mut TrackRow, _is_playing| {
                // Mutation intentionally ignored.
            },
        )
    }
}

struct PlayController;

impl<T, W> Controller<Ctx<CommonCtx, T>, W> for PlayController
where
    T: TrackIter + Data,
    W: Widget<Ctx<CommonCtx, T>>,
{
    fn event(
        &mut self,
        child: &mut W,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut Ctx<CommonCtx, T>,
        env: &Env,
    ) {
        match event {
            Event::Notification(note) => {
                if let Some(position) = note.get(cmd::PLAY_TRACK_AT) {
                    let payload = PlaybackPayload {
                        origin: data.data.origin().to_owned(),
                        tracks: data.data.tracks().to_owned(),
                        position: position.to_owned(),
                    };
                    ctx.submit_command(cmd::PLAY_TRACKS.with(payload));
                    ctx.set_handled();
                }
            }
            _ => child.event(ctx, event, data, env),
        }
    }
}

fn track_widget(display: TrackDisplay) -> impl Widget<TrackRow> {
    let mut major = Flex::row();
    let mut minor = Flex::row();

    if display.number {
        let track_number = Label::dynamic(|tr: &TrackRow, _| tr.track.track_number.to_string())
            .with_text_size(theme::TEXT_SIZE_SMALL)
            .with_text_color(theme::PLACEHOLDER_COLOR)
            .with_text_alignment(TextAlignment::Center)
            .center()
            .fix_width(theme::grid(2.0));
        major.add_child(track_number);
        major.add_default_spacer();
    }

    if display.title {
        let track_name = Label::raw()
            .with_font(theme::UI_FONT_MEDIUM)
            .lens(TrackRow::track.then(Track::name.in_arc()));
        major.add_child(track_name);
    }

    if display.artist {
        let track_artist = Label::dynamic(|tr: &TrackRow, _| tr.track.artist_name())
            .with_text_size(theme::TEXT_SIZE_SMALL);
        minor.add_child(track_artist);
    }

    if display.album {
        let track_album = Label::dynamic(|tr: &TrackRow, _| tr.track.album_name())
            .with_text_size(theme::TEXT_SIZE_SMALL)
            .with_text_color(theme::PLACEHOLDER_COLOR);
        if display.artist {
            minor.add_default_spacer();
        }
        minor.add_child(track_album);
    }

    let line_painter = Painter::new(move |ctx, is_playing: &bool, env| {
        const STYLE: StrokeStyle = StrokeStyle::new().dash_pattern(&[1.0, 2.0]);

        let line = Line::new((0.0, 0.0), (ctx.size().width, 0.0));
        let color = if *is_playing {
            env.get(theme::GREY_200)
        } else {
            env.get(theme::GREY_500)
        };
        ctx.stroke_styled(line, &color, 1.0, &STYLE);
    })
    .lens(TrackRow::is_playing())
    .fix_height(1.0);
    major.add_default_spacer();
    major.add_flex_child(line_painter, 1.0);

    if display.popularity {
        let track_popularity = Label::dynamic(|tr: &TrackRow, _| {
            tr.track
                .popularity
                .map(popularity_stars)
                .unwrap_or_default()
        })
        .with_text_size(theme::TEXT_SIZE_SMALL)
        .with_text_color(theme::PLACEHOLDER_COLOR);
        major.add_default_spacer();
        major.add_child(track_popularity);
    }

    let track_duration =
        Label::dynamic(|tr: &TrackRow, _| utils::as_minutes_and_seconds(&tr.track.duration))
            .with_text_size(theme::TEXT_SIZE_SMALL)
            .with_text_color(theme::PLACEHOLDER_COLOR);
    major.add_default_spacer();
    major.add_child(track_duration);

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(major)
        .with_spacer(2.0)
        .with_child(minor)
        .padding(theme::grid(1.0))
        .link()
        .rounded(theme::BUTTON_BORDER_RADIUS)
        .on_ex_click(move |ctx, event, tr: &mut TrackRow, _| match event.button {
            MouseButton::Left => {
                ctx.submit_notification(cmd::PLAY_TRACK_AT.with(tr.position));
            }
            MouseButton::Right => {
                ctx.show_context_menu(track_menu(tr), event.window_pos);
                ctx.set_active(true);
            }
            _ => {}
        })
}

fn popularity_stars(popularity: u32) -> String {
    const COUNT: usize = 5;

    let popularity_coef = popularity as f32 / 100.0;
    let popular = (COUNT as f32 * popularity_coef).round() as usize;
    let unpopular = COUNT - popular;

    let mut stars = String::with_capacity(COUNT);
    for _ in 0..popular {
        stars.push('★');
    }
    for _ in 0..unpopular {
        stars.push('☆');
    }
    stars
}

fn track_menu(tr: &TrackRow) -> Menu<State> {
    let mut menu = Menu::empty();

    for artist_link in &tr.track.artists {
        let more_than_one_artist = tr.track.artists.len() > 1;
        let title = if more_than_one_artist {
            LocalizedString::new("menu-item-show-artist-name")
                .with_placeholder(format!("Go To Artist “{}”", artist_link.name))
        } else {
            LocalizedString::new("menu-item-show-artist").with_placeholder("Go To Artist")
        };
        menu = menu.entry(
            MenuItem::new(title)
                .command(cmd::NAVIGATE.with(Nav::ArtistDetail(artist_link.to_owned()))),
        );
    }

    if let Some(album_link) = tr.track.album.as_ref() {
        menu = menu.entry(
            MenuItem::new(
                LocalizedString::new("menu-item-show-album").with_placeholder("Go To Album"),
            )
            .command(cmd::NAVIGATE.with(Nav::AlbumDetail(album_link.to_owned()))),
        )
    }

    menu = menu.entry(
        MenuItem::new(LocalizedString::new("menu-item-copy-link").with_placeholder("Copy Link"))
            .command(cmd::COPY.with(tr.track.url())),
    );

    menu = menu.separator();

    if tr.ctx.is_track_saved(&tr.track) {
        menu = menu.entry(
            MenuItem::new(
                LocalizedString::new("menu-item-remove-from-library")
                    .with_placeholder("Remove from Library"),
            )
            .command(cmd::UNSAVE_TRACK.with(tr.track.id.clone())),
        );
    } else {
        menu = menu.entry(
            MenuItem::new(
                LocalizedString::new("menu-item-save-to-library")
                    .with_placeholder("Save to Library"),
            )
            .command(cmd::SAVE_TRACK.with(tr.track.clone())),
        );
    }

    menu
}
