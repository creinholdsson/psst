use crate::{
    cmd,
    data::{ArtistTracks, PlaylistTracks, SavedTracks, State},
    ui,
    webapi::WebApi,
    widget::remote_image,
};
use druid::{
    commands, im::Vector, image, AppDelegate, Application, Command, DelegateCtx, Env, Handled,
    ImageBuf, Target, WindowId,
};
use lru_cache::LruCache;
use std::{sync::Arc, thread};

pub struct Delegate {
    image_cache: LruCache<Arc<str>, ImageBuf>,
    main_window: Option<WindowId>,
    preferences_window: Option<WindowId>,
}

impl Delegate {
    pub fn new() -> Self {
        const IMAGE_CACHE_SIZE: usize = 256;
        let image_cache = LruCache::new(IMAGE_CACHE_SIZE);

        Self {
            image_cache,
            main_window: None,
            preferences_window: None,
        }
    }

    pub fn with_main(main_window: WindowId) -> Self {
        let mut this = Self::new();
        this.main_window.replace(main_window);
        this
    }

    pub fn with_preferences(preferences_window: WindowId) -> Self {
        let mut this = Self::new();
        this.preferences_window.replace(preferences_window);
        this
    }

    fn spawn<F, T>(&self, f: F)
    where
        F: FnOnce() -> T,
        F: Send + 'static,
        T: Send + 'static,
    {
        // TODO: Use a thread pool.
        thread::spawn(f);
    }
}

impl AppDelegate<State> for Delegate {
    fn command(
        &mut self,
        ctx: &mut DelegateCtx,
        target: Target,
        cmd: &Command,
        data: &mut State,
        _env: &Env,
    ) -> Handled {
        if cmd.is(cmd::SHOW_MAIN) {
            match self.main_window {
                Some(id) => {
                    ctx.submit_command(commands::SHOW_WINDOW.to(id));
                }
                None => {
                    let window = ui::main_window();
                    self.main_window.replace(window.id);
                    ctx.new_window(window);
                }
            }
            Handled::Yes
        } else if cmd.is(commands::SHOW_PREFERENCES) {
            match self.preferences_window {
                Some(id) => {
                    ctx.submit_command(commands::SHOW_WINDOW.to(id));
                }
                None => {
                    let window = ui::preferences_window();
                    self.preferences_window.replace(window.id);
                    ctx.new_window(window);
                }
            }
            Handled::Yes
        } else if let Some(text) = cmd.get(cmd::COPY) {
            Application::global().clipboard().put_string(&text);
            Handled::Yes
        } else if let Handled::Yes = self.command_image(ctx, target, cmd, data) {
            Handled::Yes
        } else if let Handled::Yes = self.command_playback(ctx, target, cmd, data) {
            Handled::Yes
        } else if let Handled::Yes = self.command_playlist(ctx, target, cmd, data) {
            Handled::Yes
        } else if let Handled::Yes = self.command_library(ctx, target, cmd, data) {
            Handled::Yes
        } else if let Handled::Yes = self.command_album(ctx, target, cmd, data) {
            Handled::Yes
        } else if let Handled::Yes = self.command_artist(ctx, target, cmd, data) {
            Handled::Yes
        } else if let Handled::Yes = self.command_search(ctx, target, cmd, data) {
            Handled::Yes
        } else {
            Handled::No
        }
    }

    fn window_removed(
        &mut self,
        id: WindowId,
        data: &mut State,
        _env: &Env,
        _ctx: &mut DelegateCtx,
    ) {
        if self.preferences_window == Some(id) {
            self.preferences_window.take();
            data.preferences.reset();
        }
        if self.main_window == Some(id) {
            self.main_window.take();
        }
    }
}

impl Delegate {
    fn command_image(
        &mut self,
        ctx: &mut DelegateCtx,
        target: Target,
        cmd: &Command,
        _data: &mut State,
    ) -> Handled {
        if let Some(location) = cmd.get(remote_image::REQUEST_DATA).cloned() {
            let sink = ctx.get_external_handle();
            if let Some(image_buf) = self.image_cache.get_mut(&location).cloned() {
                let payload = remote_image::ImagePayload {
                    location,
                    image_buf,
                };
                sink.submit_command(remote_image::PROVIDE_DATA, payload, target)
                    .unwrap();
            } else {
                self.spawn(move || {
                    let dyn_image = WebApi::global()
                        .get_image(&location, image::ImageFormat::Jpeg)
                        .unwrap();
                    let image_buf = ImageBuf::from_dynamic_image(dyn_image);
                    let payload = remote_image::ImagePayload {
                        location,
                        image_buf,
                    };
                    sink.submit_command(remote_image::PROVIDE_DATA, payload, target)
                        .unwrap();
                });
            }
            Handled::Yes
        } else if let Some(payload) = cmd.get(remote_image::PROVIDE_DATA).cloned() {
            self.image_cache.insert(payload.location, payload.image_buf);
            Handled::No
        } else {
            Handled::No
        }
    }

    fn command_playlist(
        &mut self,
        ctx: &mut DelegateCtx,
        _target: Target,
        cmd: &Command,
        data: &mut State,
    ) -> Handled {
        if cmd.is(cmd::SESSION_CONNECTED) {
            data.library_mut().playlists.defer_default();
            data.user_profile.defer_default();
            Handled::Yes
        } else if let Some(link) = cmd.get(cmd::LOAD_PLAYLIST_DETAIL).cloned() {
            let sink = ctx.get_external_handle();
            data.playlist.playlist.defer(link.clone());
            data.playlist.tracks.defer(link.clone());
            self.spawn(move || {
                let result = WebApi::global().get_playlist_tracks(&link.id);
                sink.submit_command(cmd::UPDATE_PLAYLIST_TRACKS, (link, result), Target::Auto)
                    .unwrap();
            });
            Handled::Yes
        } else if let Some((link, result)) = cmd.get(cmd::UPDATE_PLAYLIST_TRACKS).cloned() {
            if data.playlist.tracks.is_deferred(&link) {
                data.playlist
                    .tracks
                    .resolve_or_reject(result.map(|tracks| PlaylistTracks {
                        id: link.id,
                        name: link.name,
                        tracks,
                    }));
            }
            Handled::Yes
        } else {
            Handled::No
        }
    }

    fn command_library(
        &mut self,
        ctx: &mut DelegateCtx,
        _target: Target,
        cmd: &Command,
        data: &mut State,
    ) -> Handled {
        if cmd.is(cmd::LOAD_SAVED_TRACKS) {
            if data.library.saved_tracks.is_empty() || data.library.saved_tracks.is_rejected() {
                data.library_mut().saved_tracks.defer_default();
                let sink = ctx.get_external_handle();
                self.spawn(move || {
                    let result = WebApi::global().get_saved_tracks();
                    sink.submit_command(cmd::UPDATE_SAVED_TRACKS, result, Target::Auto)
                        .unwrap();
                });
            }
            Handled::Yes
        } else if cmd.is(cmd::LOAD_SAVED_ALBUMS) {
            if data.library.saved_albums.is_empty() || data.library.saved_albums.is_rejected() {
                data.library_mut().saved_albums.defer_default();
                let sink = ctx.get_external_handle();
                self.spawn(move || {
                    let result = WebApi::global().get_saved_albums();
                    sink.submit_command(cmd::UPDATE_SAVED_ALBUMS, result, Target::Auto)
                        .unwrap();
                });
            }
            Handled::Yes
        } else if let Some(result) = cmd.get(cmd::UPDATE_SAVED_TRACKS).cloned() {
            match result {
                Ok(tracks) => {
                    data.common_ctx.set_saved_tracks(&tracks);
                    data.library_mut()
                        .saved_tracks
                        .resolve(SavedTracks { tracks });
                }
                Err(err) => {
                    data.common_ctx.set_saved_tracks(&Vector::new());
                    data.library_mut().saved_tracks.reject(err);
                }
            };
            Handled::Yes
        } else if let Some(result) = cmd.get(cmd::UPDATE_SAVED_ALBUMS).cloned() {
            match result {
                Ok(albums) => {
                    data.common_ctx.set_saved_albums(&albums);
                    data.library_mut().saved_albums.resolve(albums);
                }
                Err(err) => {
                    data.common_ctx.set_saved_albums(&Vector::new());
                    data.library_mut().saved_albums.reject(err);
                }
            };
            Handled::Yes
        } else if let Some(track) = cmd.get(cmd::SAVE_TRACK).cloned() {
            let track_id = track.id.to_base62();
            data.save_track(track);
            self.spawn(move || {
                let result = WebApi::global().save_track(&track_id);
                if result.is_err() {
                    // TODO: Refresh saved tracks.
                }
            });
            Handled::Yes
        } else if let Some(track_id) = cmd.get(cmd::UNSAVE_TRACK).cloned() {
            data.unsave_track(&track_id);
            self.spawn(move || {
                let result = WebApi::global().unsave_track(&track_id.to_base62());
                if result.is_err() {
                    // TODO: Refresh saved tracks.
                }
            });
            Handled::Yes
        } else if let Some(album) = cmd.get(cmd::SAVE_ALBUM).cloned() {
            let album_id = album.id.clone();
            data.save_album(album);
            self.spawn(move || {
                let result = WebApi::global().save_album(&album_id);
                if result.is_err() {
                    // TODO: Refresh saved albums.
                }
            });
            Handled::Yes
        } else if let Some(link) = cmd.get(cmd::UNSAVE_ALBUM).cloned() {
            data.unsave_album(&link.id);
            self.spawn(move || {
                let result = WebApi::global().unsave_album(&link.id);
                if result.is_err() {
                    // TODO: Refresh saved albums.
                }
            });
            Handled::Yes
        } else {
            Handled::No
        }
    }

    fn command_album(
        &mut self,
        ctx: &mut DelegateCtx,
        _target: Target,
        cmd: &Command,
        data: &mut State,
    ) -> Handled {
        if let Some(link) = cmd.get(cmd::LOAD_ALBUM_DETAIL).cloned() {
            data.album.album.defer(link.clone());
            let sink = ctx.get_external_handle();
            self.spawn(move || {
                let result = WebApi::global().get_album(&link.id);
                sink.submit_command(cmd::UPDATE_ALBUM_DETAIL, (link, result), Target::Auto)
                    .unwrap();
            });
            Handled::Yes
        } else if let Some((link, result)) = cmd.get(cmd::UPDATE_ALBUM_DETAIL).cloned() {
            if data.album.album.is_deferred(&link) {
                data.album.album.resolve_or_reject(result);
            }
            Handled::Yes
        } else {
            Handled::No
        }
    }

    fn command_artist(
        &mut self,
        ctx: &mut DelegateCtx,
        _target: Target,
        cmd: &Command,
        data: &mut State,
    ) -> Handled {
        if let Some(album_link) = cmd.get(cmd::LOAD_ARTIST_DETAIL) {
            // Load artist detail
            data.artist.artist.defer(album_link.clone());
            let link = album_link.clone();
            let sink = ctx.get_external_handle();
            self.spawn(move || {
                let result = WebApi::global().get_artist(&link.id);
                sink.submit_command(cmd::UPDATE_ARTIST_DETAIL, (link, result), Target::Auto)
                    .unwrap();
            });
            // Load artist top tracks
            data.artist.top_tracks.defer(album_link.clone());
            let link = album_link.clone();
            let sink = ctx.get_external_handle();
            self.spawn(move || {
                let result = WebApi::global().get_artist_top_tracks(&link.id);
                sink.submit_command(cmd::UPDATE_ARTIST_TOP_TRACKS, (link, result), Target::Auto)
                    .unwrap();
            });
            // Load artist's related artists
            data.artist.related_artists.defer(album_link.clone());
            let link = album_link.clone();
            let sink = ctx.get_external_handle();
            self.spawn(move || {
                let result = WebApi::global().get_related_artists(&link.id);
                sink.submit_command(cmd::UPDATE_ARTIST_RELATED, (link, result), Target::Auto)
                    .unwrap();
            });
            // Load artist albums
            data.artist.albums.defer(album_link.clone());
            let link = album_link.clone();
            let sink = ctx.get_external_handle();
            self.spawn(move || {
                let result = WebApi::global().get_artist_albums(&link.id);
                sink.submit_command(cmd::UPDATE_ARTIST_ALBUMS, (link, result), Target::Auto)
                    .unwrap();
            });
            Handled::Yes
        } else if let Some((link, result)) = cmd.get(cmd::UPDATE_ARTIST_DETAIL).cloned() {
            if data.artist.artist.is_deferred(&link) {
                data.artist.artist.resolve_or_reject(result);
            }
            Handled::Yes
        } else if let Some((link, result)) = cmd.get(cmd::UPDATE_ARTIST_ALBUMS).cloned() {
            if data.artist.albums.is_deferred(&link) {
                data.artist.albums.resolve_or_reject(result);
            }
            Handled::Yes
        } else if let Some((link, result)) = cmd.get(cmd::UPDATE_ARTIST_TOP_TRACKS).cloned() {
            if data.artist.top_tracks.is_deferred(&link) {
                data.artist
                    .top_tracks
                    .resolve_or_reject(result.map(|tracks| ArtistTracks {
                        id: link.id,
                        name: link.name,
                        tracks,
                    }));
            }
            Handled::Yes
        } else if let Some((link, result)) = cmd.get(cmd::UPDATE_ARTIST_RELATED).cloned() {
            if data.artist.related_artists.is_deferred(&link) {
                data.artist.related_artists.resolve_or_reject(result);
            }
            Handled::Yes
        } else {
            Handled::No
        }
    }

    fn command_search(
        &mut self,
        ctx: &mut DelegateCtx,
        _target: Target,
        cmd: &Command,
        data: &mut State,
    ) -> Handled {
        if let Some(query) = cmd.get(cmd::LOAD_SEARCH_RESULTS).cloned() {
            let sink = ctx.get_external_handle();
            data.search.results.defer(query.clone());
            self.spawn(move || {
                let result = WebApi::global().search(&query);
                sink.submit_command(cmd::UPDATE_SEARCH_RESULTS, result, Target::Auto)
                    .unwrap();
            });
            Handled::Yes
        } else if let Some(result) = cmd.get(cmd::UPDATE_SEARCH_RESULTS).cloned() {
            data.search.results.resolve_or_reject(result);
            Handled::Yes
        } else {
            Handled::No
        }
    }

    fn command_playback(
        &mut self,
        ctx: &mut DelegateCtx,
        _target: Target,
        cmd: &Command,
        data: &mut State,
    ) -> Handled {
        if cmd.is(cmd::PLAYBACK_PLAYING) {
            let (item, _progress) = cmd.get_unchecked(cmd::PLAYBACK_PLAYING);

            data.playback.now_playing.as_mut().map(|current| {
                current.analysis.defer(item.clone());
            });
            let item = item.clone();
            let sink = ctx.get_external_handle();
            self.spawn(move || {
                let result = WebApi::global().get_audio_analysis(&item.to_base62());
                sink.submit_command(cmd::UPDATE_AUDIO_ANALYSIS, (item, result), Target::Auto)
                    .unwrap();
            });

            Handled::No
        } else {
            Handled::No
        }
    }
}
