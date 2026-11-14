//! The one test that really talks to Jamendo. Ignored by default and skipped without a
//! key, so the suite stays offline unless someone asks for this on purpose:
//!
//! ```sh
//! LUMEN_JAMENDO_CLIENT_ID=<your free client id> \
//!   cargo test -p lumen --test music_live -- --ignored --nocapture
//! ```
//!
//! It proves the three things a fixture cannot: that the URL we build is one Jamendo
//! accepts, that what comes back parses into tracks we may legally put under a video, and
//! that a download lands in `<library>/music/` with the sidecar that credits it.

use lumen_lib::music::{self, MusicQuery, MusicWant};

fn query(text: &str) -> MusicQuery {
    MusicQuery {
        text: Some(text.into()),
        vibe: None,
        genre: None,
        instrumental_only: false,
        tempo: None,
        include_no_derivatives: false,
        limit: 10,
        ..MusicQuery::default()
    }
}

#[tokio::test]
#[ignore = "talks to Jamendo; needs LUMEN_JAMENDO_CLIENT_ID"]
async fn a_real_search_returns_music_that_may_be_used() {
    let Ok(key) = std::env::var("LUMEN_JAMENDO_CLIENT_ID") else {
        eprintln!("set LUMEN_JAMENDO_CLIENT_ID to run this");
        return;
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(concat!("Lumen/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap();

    let tracks = music::search(&client, &key, &query("piano")).await.expect("the search");
    assert!(!tracks.is_empty(), "Jamendo found no piano at all");
    assert!(
        tracks.iter().all(music::usable),
        "a no-derivatives track reached the picker: {:?}",
        tracks.iter().find(|t| !music::usable(t)).map(|t| &t.licence_name)
    );
    for t in tracks.iter().take(3) {
        println!("{} — {} [{}]", t.title, t.artist, t.licence_name);
    }

    // And one of them into a library, exactly as the picker does it.
    let dir = tempfile::tempdir().unwrap();
    let track = tracks.into_iter().next().unwrap();
    let saved = music::download(&client, &track, dir.path()).await.expect("the download");

    let bytes = std::fs::metadata(&saved.path).expect("the audio file").len();
    assert!(bytes > 100_000, "{} is only {bytes} bytes", saved.path);
    assert!(saved.credit.contains(&track.artist), "the credit names the artist: {}", saved.credit);

    let sidecar = std::path::Path::new(&saved.path).with_extension("json");
    let read_back = music::sidecar_of(std::path::Path::new(&saved.path)).expect("the sidecar");
    assert!(sidecar.exists());
    assert_eq!(read_back.credit, saved.credit);
    println!("saved {} ({bytes} bytes)\n{}", saved.title, saved.credit);
}

/// Round 4F: automatic music for three kinds of show, searched across the whole catalogue
/// and downloaded into a temporary library.
#[tokio::test]
#[ignore = "talks to Jamendo; needs LUMEN_JAMENDO_CLIENT_ID"]
async fn automatic_music_comes_from_the_whole_catalogue() {
    let Ok(key) = std::env::var("LUMEN_JAMENDO_CLIENT_ID") else {
        eprintln!("set LUMEN_JAMENDO_CLIENT_ID to run this");
        return;
    };
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().unwrap();
    let jamendo = music::Jamendo { client: &client, key: &key };
    let dir = tempfile::tempdir().unwrap();
    // Listening needs FFmpeg; without it the choice falls back to tags, which still passes.
    let ff = photos_media::Ffmpeg::locate();
    let listen_dir = tempfile::tempdir().unwrap();
    let counter = std::sync::atomic::AtomicU32::new(0);
    let analyse = |bytes: Vec<u8>| {
        let ff = ff.clone();
        let path = listen_dir.path().join(format!("{}.mp3", counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        async move {
            let ff = ff?;
            std::fs::write(&path, bytes).ok()?;
            let (grid, energy) = photos_media::beats::detect(&ff, &path).ok()?;
            let rate = photos_media::beats::DSP_SAMPLE_RATE;
            let samples = photos_media::beats::decode_mono(&ff, &path, rate).ok()?;
            Some(photos_core::music_profile::profile(&samples, rate, &grid, energy))
        }
    };
    let mut in_use = std::collections::HashSet::new();
    for (name, moods, energy) in [("calm", ["calm", "dreamy"], 0.15), ("energetic", ["energetic", "happy"], 0.8), ("romantic", ["romantic", "calm"], 0.3)] {
        let want = MusicWant { moods: music::weighted(&moods), energy: Some(energy), target_ms: 90_000, seed: 7 };
        let all = music::search_many(&jamendo, &music::queries_for(&want)).await.expect("Jamendo answered");
        let t = std::time::Instant::now();
        let got = music::music_for(Some(&jamendo), dir.path(), &want, &in_use, |_| None, analyse).await.expect("music suits");
        let window = got.profile.as_ref().map(|p| photos_core::music_profile::choose_window(p, want.target_ms));
        let track = got.track;
        println!(
            "{name}: {} candidates; chose {} — {} ({} s) in {:?}; heard: {:?}; plays from {:?}",
            all.len(),
            track.title,
            track.artist,
            track.duration_ms / 1000,
            t.elapsed(),
            got.profile.as_ref().map(|p| (p.tempo, p.energy, p.brightness, p.dynamics)),
            window.map(|w| w.start_ms)
        );
        assert!(std::path::Path::new(&track.path).is_file());
        assert!(!track.credit.is_empty());
        in_use.insert(music::file_name_of(&track));
    }
    assert_eq!(music::recently_used(dir.path()).len(), 3, "three different tracks");
}
