//! Reads the Apple Music and Spotify sessions from Windows' media controls
//! (SMTC).
//!
//! Every other media session (browsers, web players, games, video players,
//! ...) is skipped by an exact app-identity check, so nothing else can ever
//! reach Discord. `GetCurrentSession()` is deliberately never used: it
//! returns whichever app Windows considers "current", which may be any app.

use crate::prelude::*;
use crate::sys::{self, EPOCH_DIFF};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};

/// AUMID ("<package family>!<app>") of the Microsoft Store Apple Music app.
/// The package suffix is Apple's publisher hash. Matched in full: Apple TV
/// shares the publisher hash and the "!App" part, iTunes/Cider/browsers/the
/// web player all use other ids.
const APPLE_MUSIC_AUMID: &str = "AppleInc.AppleMusicWin_nzyj5cx40ttqa!App";
/// Spotify's desktop app (a plain exe, so its id is the file name) and its
/// Microsoft Store package. The web player plays inside a browser, whose id
/// is the browser's.
const SPOTIFY_AUMIDS: [&str; 2] = ["Spotify.exe", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Player {
    AppleMusic,
    Spotify,
}

impl Player {
    pub const ALL: [Player; 2] = [Player::AppleMusic, Player::Spotify];

    pub fn name(self) -> &'static str {
        match self {
            Player::AppleMusic => "Apple Music",
            Player::Spotify => "Spotify",
        }
    }

    /// The player a media session belongs to, by its exact app id.
    pub fn of(aumid: &str) -> Option<Player> {
        if aumid.eq_ignore_ascii_case(APPLE_MUSIC_AUMID) {
            Some(Player::AppleMusic)
        } else if SPOTIFY_AUMIDS.iter().any(|id| aumid.eq_ignore_ascii_case(id)) {
            Some(Player::Spotify)
        } else {
            None
        }
    }
}

/// Waits for a WinRT async result, giving up after 3 s. Not `join()`: in
/// windows 0.62 that can block forever on an error, which would leave a
/// stale song on the status.
macro_rules! wait {
    ($op:expr) => {{
        let op = $op;
        let deadline = sys::ticks() + 3000;
        loop {
            let status = op.Status()?.0; // 0 started, 1 completed, 2 canceled, 3 error
            if status == 1 {
                break op.GetResults();
            }
            if status != 0 || sys::ticks() > deadline {
                let _ = op.Cancel();
                break Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
            }
            sys::sleep(1);
        }
    }};
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Playing,
    Paused,
    /// Between tracks / buffering: keep whatever is currently shown.
    Changing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub player: Player,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// A podcast episode (Spotify): `artist` is the show, and there's no album.
    pub episode: bool,
    pub state: State,
    /// Track length in ms, 0 when unknown (e.g. radio).
    pub duration_ms: i64,
    /// Playback position in ms, extrapolated to "now" while playing.
    pub position_ms: i64,
}

const DASH: char = '\u{2014}';
const STATION_SUFFIXES: [&str; 7] =
    ["\u{2019}s Station", "'s Station", "s Sender", "さんのステーション", "的电台", "的電台", "의 스테이션"];
const STATION_PREFIXES: [&str; 7] =
    ["Emisora de ", "Estación de ", "Estação de ", "Station de : ", "Station för ", "Station van ", "Stazione di "];

/// Personal radio stations are named after the listener ("John Doe’s
/// Station"), which would put the user's own name on their status. These are
/// the localised forms Apple Music uses.
fn is_station(s: &str) -> bool {
    let s = s.trim();
    STATION_SUFFIXES.iter().any(|x| s.len() > x.len() && s.ends_with(x))
        || STATION_PREFIXES.iter().any(|x| s.len() > x.len() && s.starts_with(x))
        || (s.starts_with("Моя станция (") && s.ends_with(')'))
}

fn mentions_station(s: &str) -> bool {
    s.split(DASH).any(is_station) || STATION_SUFFIXES.iter().any(|x| s.find(x).is_some_and(|i| i > 0))
}

/// Cleans Apple Music's raw SMTC fields into (title, artist, album), or
/// `None` for "don't show this".
///
/// Apple Music on Windows puts "Artist — Album" in the Artist field and
/// leaves AlbumTitle empty; while a personal station plays it becomes
/// "Artist — Album — John Doe’s Station". This fails closed: only the
/// first two parts are ever used, so station names in any language stay off
/// the status. A title without an artist is a placeholder ("Connecting…",
/// a station loading) and is never shown.
pub fn tidy(title: &str, artist: &str, album_title: &str) -> Option<(String, String, String)> {
    let title = title.trim();
    if title.is_empty() || mentions_station(title) {
        return None;
    }
    let mut parts = artist.split(DASH).map(str::trim);
    let artist = parts.next().unwrap_or("");
    if artist.is_empty() || is_station(artist) {
        return None;
    }
    let album = if album_title.trim().is_empty() { parts.next() } else { album_title.split(DASH).next() };
    let album = album.map(str::trim).filter(|a| !is_station(a)).unwrap_or("");
    Some((title.to_string(), artist.to_string(), album.to_string()))
}

/// Spotify's fields are already clean (Title, Artist, AlbumTitle), as
/// (title, artist, album, episode). A podcast episode has no artist and its
/// show in AlbumTitle: the show stands in for the artist. `None` for ads and
/// for the idle "Spotify" placeholder, which have neither.
pub fn tidy_spotify(title: &str, artist: &str, album: &str) -> Option<(String, String, String, bool)> {
    let (title, artist, album) = (title.trim(), artist.trim(), album.trim());
    let episode = artist.is_empty();
    let (artist, album) = if episode { (album, "") } else { (artist, album) };
    if title.is_empty() || artist.is_empty() || artist.eq_ignore_ascii_case("Spotify") {
        return None;
    }
    if ["Advertisement", "Spotify", "Spotify Free", "Spotify Premium"].iter().any(|x| title.eq_ignore_ascii_case(x)) {
        return None;
    }
    Some((title.to_string(), artist.to_string(), album.to_string(), episode))
}

/// The session to show when several players have one: playing beats
/// switching tracks beats paused; on a tie the player shown last stays,
/// else Apple Music.
pub fn pick(tracks: impl IntoIterator<Item = Track>, last: Option<Player>) -> Option<Track> {
    let rank = |t: &Track| {
        let state = match t.state {
            State::Playing => 2,
            State::Changing => 1,
            State::Paused => 0,
        };
        (state, Some(t.player) == last, t.player == Player::AppleMusic)
    };
    let mut best: Option<Track> = None;
    for t in tracks {
        if best.as_ref().is_none_or(|b| rank(&t) > rank(b)) {
            best = Some(t);
        }
    }
    best
}

/// How long a doubtful "Paused" counts as playing: after a skip, or after the
/// position was last seen moving. Players update the timeline every ~4 s.
const DOUBT_MS: i64 = 6000;

/// Whether a player is really paused. Spotify sometimes goes on saying
/// "Paused" after skipping to the next song while that song plays (seen for
/// 8 s), which would take the status down until it corrects itself. So
/// "Paused" right after a skip from a playing song is doubted, and so is
/// "Paused" while the position keeps moving between timeline updates; a
/// position standing still settles it. A plain pause is believed at once.
#[derive(Clone, Default)]
pub struct Liveness {
    /// Title and artist of the last reading.
    song: String,
    /// What the last reading was judged to be.
    playing: bool,
    /// Position and time (Unix ms) of the last timeline update, and whether
    /// the player said "Paused" then.
    at: (i64, i64, bool),
    doubt_until: i64,
}

impl Liveness {
    /// Judges one reading (`reported` is Playing or Paused; `pos` is the
    /// position at the timeline update from `updated`, 0 if unknown).
    /// Returns whether the song is playing.
    pub fn judge(&mut self, song: &str, reported: State, pos: i64, updated: i64, now: i64) -> bool {
        let paused = reported == State::Paused;
        let fresh = updated > 0 && updated != self.at.1;
        if !paused {
            self.doubt_until = 0;
        } else if song != self.song {
            // Skipping on from a playing song keeps playing.
            self.doubt_until = if self.playing { now + DOUBT_MS } else { 0 };
        } else if fresh && self.at.2 {
            // Two updates, both while "Paused": did the position move? (Too
            // close together, a playing song barely moves: no verdict.)
            let (moved, took) = (pos - self.at.0, updated - self.at.1);
            if moved >= 1000 && (moved - took).abs() <= 1500 {
                self.doubt_until = updated + DOUBT_MS;
            } else if took >= 2000 && moved.abs() < 500 {
                self.doubt_until = 0;
            }
        }
        if fresh || song != self.song {
            self.at = (pos, updated, paused);
        }
        self.song = song.to_string();
        self.playing = !paused || now < self.doubt_until;
        self.playing
    }
}

#[derive(Default)]
pub struct Smtc {
    mgr: Option<Manager>,
    /// The player picked last time (see `pick`).
    last: Option<Player>,
    /// Per player (by `Player::ALL` index).
    live: [Liveness; 2],
}

impl Smtc {
    /// What the enabled `players` are playing; `Ok(None)` when none of them
    /// has an active session.
    pub fn poll(&mut self, players: &[Player]) -> windows::core::Result<Option<Track>> {
        let r = self.poll_inner(players);
        match &r {
            Ok(Some(t)) => self.last = Some(t.player),
            Ok(None) => {}
            Err(_) => self.mgr = None, // re-acquire on the next poll
        }
        r
    }

    fn poll_inner(&mut self, players: &[Player]) -> windows::core::Result<Option<Track>> {
        if self.mgr.is_none() {
            self.mgr = Some(wait!(Manager::RequestAsync()?)?);
        }
        let sessions = self.mgr.as_ref().unwrap().GetSessions()?;
        // Each player's session (by `Player::ALL` index), and whether a second
        // one claimed to be that player: Windows doesn't verify session ids,
        // so one of them is an impostor, and that player is skipped.
        let mut found: [Option<Session>; 2] = [None, None];
        let mut impostor = [false; 2];
        for i in 0..sessions.Size()? {
            let s = sessions.GetAt(i)?;
            let Some(p) = Player::of(&s.SourceAppUserModelId()?.to_string_lossy()) else { continue };
            if players.contains(&p) {
                impostor[p as usize] |= found[p as usize].is_some();
                found[p as usize] = Some(s);
            }
        }
        let mut tracks = [None, None];
        let mut error = None;
        let now = sys::now_ms();
        for (i, p) in Player::ALL.into_iter().enumerate() {
            match found[i].as_ref().filter(|_| !impostor[i]).map(|s| read(p, s)) {
                Some(Ok(Some((mut t, updated)))) => {
                    if t.state != State::Changing {
                        let song = format!("{}\0{}", t.title, t.artist);
                        if self.live[i].judge(&song, t.state, t.position_ms, updated, now) {
                            t.state = State::Playing;
                        }
                    }
                    if t.state == State::Playing && updated > 0 && (0..24 * 3600 * 1000).contains(&(now - updated)) {
                        t.position_ms += now - updated; // where it is by now
                    }
                    if t.duration_ms > 0 {
                        t.position_ms = t.position_ms.min(t.duration_ms);
                    }
                    tracks[i] = Some(t);
                }
                Some(Ok(None)) | None => self.live[i] = Liveness::default(), // gone or stopped
                Some(Err(e)) => error = Some(e),
            }
        }
        // A player that couldn't be read might be the one playing: fail
        // closed (show nothing) unless another one definitely is.
        match (pick(tracks.into_iter().flatten(), self.last), error) {
            (Some(t), _) if t.state == State::Playing => Ok(Some(t)),
            (_, Some(e)) => Err(e),
            (t, None) => Ok(t),
        }
    }

    /// Drops what's known about the players (while the status is hidden or
    /// every player is off, nothing is read, so it would be out of date).
    pub fn forget(&mut self) {
        self.live = Default::default();
    }

    /// Diagnostic listing of every session's app id (used by `--dump`).
    pub fn session_ids(&mut self) -> Vec<String> {
        let _ = self.poll(&[]);
        let Some(mgr) = &self.mgr else { return Vec::new() };
        let Ok(sessions) = mgr.GetSessions() else { return Vec::new() };
        (0..sessions.Size().unwrap_or(0))
            .filter_map(|i| sessions.GetAt(i).ok()?.SourceAppUserModelId().ok())
            .map(|h| h.to_string_lossy())
            .collect()
    }
}

/// One reading of a session, and when (Unix ms, 0 if unknown) its timeline
/// was last updated; the track's position is the one at that update.
fn read(player: Player, s: &Session) -> windows::core::Result<Option<(Track, i64)>> {
    let state = match s.GetPlaybackInfo()?.PlaybackStatus()? {
        Status::Playing => State::Playing,
        Status::Paused => State::Paused,
        Status::Changing | Status::Opened => State::Changing,
        _ => return Ok(None), // Stopped / Closed
    };
    let props = wait!(s.TryGetMediaPropertiesAsync()?)?;
    let (title, artist, album) =
        (props.Title()?.to_string_lossy(), props.Artist()?.to_string_lossy(), props.AlbumTitle()?.to_string_lossy());
    let fields = match player {
        Player::AppleMusic => tidy(&title, &artist, &album).map(|(t, a, al)| (t, a, al, false)),
        Player::Spotify => tidy_spotify(&title, &artist, &album),
    };
    // Placeholders (loading, station names, ads) are treated as a
    // transition; the worker clears the presence if they persist.
    let Some((title, artist, album, episode)) = fields else { return Ok(Some((changing(player), 0))) };

    let tl = s.GetTimelineProperties()?;
    let start = tl.StartTime()?.Duration;
    let duration_ms = ((tl.EndTime()?.Duration - start) / 10_000).max(0);
    let position_ms = ((tl.Position()?.Duration - start) / 10_000).max(0);
    let updated = tl.LastUpdatedTime()?.UniversalTime;
    let updated_ms = if updated > EPOCH_DIFF { (updated - EPOCH_DIFF) / 10_000 } else { 0 };
    Ok(Some((Track { player, title, artist, album, episode, state, duration_ms, position_ms }, updated_ms)))
}

fn changing(player: Player) -> Track {
    Track {
        player,
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        episode: false,
        state: State::Changing,
        duration_ms: 0,
        position_ms: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity() {
        let apple = Some(Player::AppleMusic);
        let spotify = Some(Player::Spotify);
        assert_eq!(Player::of("AppleInc.AppleMusicWin_nzyj5cx40ttqa!App"), apple);
        assert_eq!(Player::of("appleinc.applemusicwin_NZYJ5CX40TTQA!APP"), apple);
        assert_eq!(Player::of("Spotify.exe"), spotify);
        assert_eq!(Player::of("spotify.EXE"), spotify);
        assert_eq!(Player::of("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), spotify);
        for other in [
            "Chrome",
            "msedge.exe",
            "firefox.exe",
            "Spotify",
            "Spotify.exe ",
            "SpotifyWebHelper.exe",
            "Spotify.exe.evil",
            "C:\\Evil\\Spotify.exe",
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0",
            "SpotifyAB.SpotifyMusic_evil!Spotify",
            "open.spotify.com-8B0B9F5_7vh1tm7h3g5s0!App", // web player PWA
            "308046B0AF4A39CB",                           // Firefox
            "AppleMusic.exe",
            "iTunes.exe",
            "AppleInc.iTunes_nzyj5cx40ttqa!iTunes",
            "music.apple.com-A5F611C_7vh1tm7h3g5s0!App", // web player PWA
            "electron.app.Apple Music Beta",
            "CiderCollective.Cider_a6qxe093bx5xj!App",
            "AppleInc.AppleMusicWin_nzyj5cx40ttqa!LibraryServer",
            " AppleInc.AppleMusicWin_nzyj5cx40ttqa!App",
            "AppleInc.AppleMusicWin_nzyj5cx40ttqa",
            "AppleInc.AppleMusicWin_nzyj5cx40ttqa!",
            "AppleInc.AppleMusicWin_nzyj5cx40ttqaX!App",
            "AppleInc.AppleMusicWin_evil!App",
            "AppleInc.AppleTVWin_nzyj5cx40ttqa!App",
            "Evil!AppleInc.AppleMusicWin_nzyj5cx40ttqa!App",
            "AppleInc.AppleMusicWin_nzyj5cx40ttqa!App!x",
            "",
        ] {
            assert_eq!(Player::of(other), None, "{other}");
        }
    }

    #[test]
    fn spotify_fields() {
        let song = |a: &str, b: &str, c: &str| Some((a.into(), b.into(), c.into(), false));
        assert_eq!(tidy_spotify(" Tal Vez ", "Paulo Londra", "Homerun"), song("Tal Vez", "Paulo Londra", "Homerun"));
        // Dashes are part of Spotify's names, not packed fields.
        assert_eq!(
            tidy_spotify("Song - Remastered 2011", "A \u{2014} B", ""),
            song("Song - Remastered 2011", "A \u{2014} B", "")
        );
        // A podcast episode, as Spotify reports one: no artist, the show as the album.
        assert_eq!(
            tidy_spotify("La \u{da}ltima vez-Anuel x Bad Bunny ", "", "Anuel AA"),
            Some(("La \u{da}ltima vez-Anuel x Bad Bunny".into(), "Anuel AA".into(), String::new(), true))
        );
        for (title, artist) in
            [("Advertisement", "Brand"), ("Spotify", ""), ("Spotify Free", "Spotify"), ("Song", ""), ("", "Artist")]
        {
            assert_eq!(tidy_spotify(title, artist, ""), None, "{title} / {artist}");
        }
        assert_eq!(tidy_spotify("Advertisement", "", "Brand"), None);
        assert_eq!(tidy_spotify("Spotify", "", "Spotify"), None);
    }

    /// The sequences recorded from Spotify, replayed (times in ms).
    #[test]
    fn doubtful_pauses() {
        use State::{Paused, Playing};
        let mut l = Liveness::default();
        // Playing, then skipped: the next song says "Paused" but plays on,
        // its position moving between the timeline updates ~4 s apart.
        assert!(l.judge("A", Playing, 11_000, 1_000, 1_500));
        assert!(l.judge("A", Playing, 15_500, 5_500, 6_500));
        assert!(l.judge("B", Paused, 0, 7_100, 7_200), "skip from a playing song");
        assert!(l.judge("B", Paused, 0, 7_100, 10_000));
        assert!(l.judge("B", Paused, 3_300, 10_400, 11_000), "the position moved");
        assert!(l.judge("B", Paused, 7_800, 14_900, 15_000));
        assert!(l.judge("B", Paused, 7_800, 14_900, 20_000), "still within 6 s of the last move");
        assert!(!l.judge("B", Paused, 7_800, 14_900, 21_000), "no update for 6 s: believed");
        assert!(l.judge("B", Playing, 8_000, 21_500, 21_600));

        // A plain pause is believed straight away, and the periodic updates
        // while paused (same position) keep it paused.
        let mut l = Liveness::default();
        assert!(l.judge("C", Playing, 6_490, 8_950, 9_000));
        assert!(!l.judge("C", Paused, 6_910, 9_610, 9_700), "paused");
        assert!(!l.judge("C", Paused, 6_910, 13_460, 13_500));

        // A skip that really lands paused: doubted until the next update shows
        // the position standing still.
        let mut l = Liveness::default();
        assert!(l.judge("D", Playing, 5_000, 1_000, 1_100));
        assert!(l.judge("E", Paused, 0, 2_000, 2_100));
        assert!(!l.judge("E", Paused, 0, 6_000, 6_100), "stood still");

        // Skipping while paused stays paused.
        let mut l = Liveness::default();
        assert!(!l.judge("F", Paused, 1_000, 1_000, 1_100));
        assert!(!l.judge("G", Paused, 0, 2_000, 2_100));

        // After the status was hidden (Smtc::forget), a paused song is paused.
        let mut l = Liveness::default();
        assert!(l.judge("H", Playing, 1_000, 1_000, 1_100));
        l = Liveness::default();
        assert!(!l.judge("I", Paused, 0, 50_000, 60_000));

        // Two updates close together (the new song's timeline arriving in
        // pieces) don't settle a doubtful pause either way.
        let mut l = Liveness::default();
        assert!(l.judge("J", Playing, 9_000, 1_000, 6_500));
        assert!(l.judge("K", Paused, 0, 7_100, 7_200));
        assert!(l.judge("K", Paused, 300, 7_400, 7_800), "300 ms apart: no verdict");
        assert!(l.judge("K", Paused, 3_300, 10_400, 10_500), "then it moved");
    }

    #[test]
    fn picks_the_player_in_use() {
        let tr = |p: Player, state: State| Track {
            player: p,
            title: p.name().into(),
            artist: "A".into(),
            album: String::new(),
            episode: false,
            state,
            duration_ms: 0,
            position_ms: 0,
        };
        let who = |v: Vec<Track>, last| pick(v, last).map(|t| t.player);
        use Player::*;
        use State::*;
        assert_eq!(who(vec![], None), None);
        assert_eq!(who(vec![tr(Spotify, Paused)], None), Some(Spotify));
        assert_eq!(who(vec![tr(AppleMusic, Paused), tr(Spotify, Playing)], None), Some(Spotify));
        assert_eq!(who(vec![tr(Spotify, Paused), tr(AppleMusic, Playing)], Some(Spotify)), Some(AppleMusic));
        assert_eq!(who(vec![tr(AppleMusic, Changing), tr(Spotify, Paused)], None), Some(AppleMusic));
        // Ties: the one already shown stays, else Apple Music.
        assert_eq!(who(vec![tr(Spotify, Playing), tr(AppleMusic, Playing)], None), Some(AppleMusic));
        assert_eq!(who(vec![tr(AppleMusic, Playing), tr(Spotify, Playing)], Some(Spotify)), Some(Spotify));
        assert_eq!(who(vec![tr(AppleMusic, Paused), tr(Spotify, Paused)], Some(Spotify)), Some(Spotify));
        assert_eq!(who(vec![tr(Spotify, Paused), tr(AppleMusic, Paused)], Some(AppleMusic)), Some(AppleMusic));
    }

    fn t(title: &str, artist: &str, album: &str) -> Option<(String, String, String)> {
        tidy(title, artist, album)
    }
    fn some(a: &str, b: &str, c: &str) -> Option<(String, String, String)> {
        Some((a.into(), b.into(), c.into()))
    }

    #[test]
    fn split() {
        assert_eq!(
            t("GIMME A HUG", "Drake \u{2014} $ome $exy $ongs 4 U", ""),
            some("GIMME A HUG", "Drake", "$ome $exy $ongs 4 U")
        );
        assert_eq!(t("S", "A \u{2014} B \u{2014} C", ""), some("S", "A", "B"));
        assert_eq!(t("S", "A\u{2014}B", ""), some("S", "A", "B"));
        assert_eq!(t("S", "Solo", ""), some("S", "Solo", ""));
        assert_eq!(t(" S ", "X - Y", "Album"), some("S", "X - Y", "Album"));
        assert_eq!(t("", "A", ""), None);
    }

    #[test]
    fn personal_stations_never_leak() {
        let it = "It's My Life";
        assert_eq!(
            t(it, "Bon Jovi \u{2014} Crush \u{2014} John Doe\u{2019}s Station", ""),
            some(it, "Bon Jovi", "Crush")
        );
        assert_eq!(t(it, "Bon Jovi \u{2014} Crush \u{2014} John's Station", ""), some(it, "Bon Jovi", "Crush"));
        assert_eq!(t(it, "Bon Jovi", "Crush \u{2014} Estación de John"), some(it, "Bon Jovi", "Crush"));
        assert_eq!(t(it, "Bon Jovi \u{2014} Crush \u{2014} Моя станция (John)", ""), some(it, "Bon Jovi", "Crush"));
        assert_eq!(t(it, "Bon Jovi \u{2014} Johns Sender", ""), some(it, "Bon Jovi", ""));
        assert_eq!(t(it, "Bon Jovi \u{2014} John さんのステーション", ""), some(it, "Bon Jovi", ""));
        assert_eq!(t("John Doe\u{2019}s Station", "", ""), None);
        assert_eq!(t("Station van John", "", ""), None);
        assert_eq!(t("Connecting\u{2026}", "", ""), None);
        assert_eq!(t("connecting...", "", ""), None);
        assert_eq!(t("Connecting", "", ""), None);
        assert_eq!(t("Station", "Artist", ""), some("Station", "Artist", ""));
        // Languages the list doesn't know are still dropped (3rd part).
        assert_eq!(t(it, "Bon Jovi \u{2014} Crush \u{2014} Johns stasjon", ""), some(it, "Bon Jovi", "Crush"));
        assert_eq!(
            t(it, "Bon Jovi \u{2014} Crush \u{2014} Stacja u\u{017c}ytkownika John", ""),
            some(it, "Bon Jovi", "Crush")
        );
        assert_eq!(t(it, "Bon Jovi \u{2014} Crush \u{2014} James\u{2019} Station", ""), some(it, "Bon Jovi", "Crush"));
        // A placeholder title with no artist is never shown, whatever it says.
        assert_eq!(t("Johns stasjon", "", ""), None);
        assert_eq!(t("John Doe\u{2019}s Station", "Bon Jovi", ""), None);
    }
}
