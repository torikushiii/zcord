use super::*;
use super::{download::*, stills::*, urls::*};

#[tokio::test]
async fn profile_styles_gif_upload_rejects_non_provider_urls_without_cache_writes() {
    let root = temp_directory("profile-gif-hosts");
    let service = Service {
        client: reqwest::Client::new(),
        directory: root.join("images"),
        video_directory: root.join("videos"),
        frames_directory: root.join("frames"),
        permits: Semaphore::new(1),
        video_permits: Semaphore::new(1),
        initialized: OnceCell::new(),
        image_written: AtomicU64::new(0),
        video_written: AtomicU64::new(0),
        frames_written: AtomicU64::new(0),
        pruning: Arc::new(AtomicBool::new(false)),
    };
    for url in [
        "not a URL",
        "https://127.0.0.1/gif",
        "https://cdn.discordapp.com/gif",
        "http://media.tenor.com/gif",
        "https://media.tenor.com.evil.test/gif",
        "https://user:pass@media.giphy.com/gif",
        "https://static.klipy.com:8443/gif",
    ] {
        assert_eq!(
            service
                .load_gif_provider_media(url, 10 * 1024 * 1024)
                .await
                .unwrap_err(),
            "Choose a GIF from Tenor, Giphy, or Klipy.",
            "{url}"
        );
    }
    assert!(!root.exists());
}

#[tokio::test]
async fn profile_styles_gif_download_limits_declared_and_streamed_bytes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for (reply, limit, expected) in [
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd",
            5,
            Ok(b"abcd".to_vec()),
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd",
            4,
            Err("Choose a GIF smaller than 10 MB.".to_owned()),
        ),
        (
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nab\r\n2\r\ncd\r\n0\r\n\r\n",
            4,
            Err("Choose a GIF smaller than 10 MB.".to_owned()),
        ),
        (
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nab\r\n2\r\ncd\r\n0\r\n\r\n",
            5,
            Ok(b"abcd".to_vec()),
        ),
        (
            "HTTP/1.1 302 Found\r\nContent-Length: 0\r\nLocation: https://evil.test/gif\r\nConnection: close\r\n\r\n",
            5,
            Err("Could not download this GIF.".to_owned()),
        ),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/gif", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            let mut buffer = [0; 1024];
            while !headers.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                headers.extend_from_slice(&buffer[..read]);
            }
            let _ = stream.write_all(reply.as_bytes()).await;
        });
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap();
        assert_eq!(gif_provider_bytes(response, limit).await, expected);
        server.await.unwrap();
    }
}

fn temp_directory(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "zcord-image-{name}-test-{:016x}",
        rand::random::<u64>()
    ))
}

fn set_age(path: &Path, age: Duration) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - age)
        .unwrap();
}

#[test]
fn video_specs_resolve_into_videos_dir() {
    let images = Path::new("images");
    let videos = Path::new("videos");
    let video = ImageSpec::Video {
        url: "https://cdn.discordapp.com/attachments/1/2/video.mp4".into(),
        width: 640,
        height: 360,
    };
    assert_eq!(cache_directory(images, videos, &video), videos);
    let poster = ImageSpec::VideoPoster {
        id: 1,
        url: "https://cdn.discordapp.com/attachments/1/2/poster.jpg".into(),
        width: 640,
        height: 360,
    };
    assert_eq!(cache_directory(images, videos, &poster), images);
}

#[tokio::test]
async fn video_cache_hit_uses_videos_dir_and_refreshes_mtime() {
    let root = temp_directory("video-hit");
    let videos = root.join("videos");
    std::fs::create_dir_all(&videos).unwrap();
    let video = ImageSpec::Video {
        url: "https://cdn.discordapp.com/attachments/1/2/video.mp4".into(),
        width: 640,
        height: 360,
    };
    let path = videos.join(file_name(&video, 64));
    std::fs::write(&path, b"\0\0\0\x18ftypisom\0\0\0\0").unwrap();
    set_age(&path, Duration::from_secs(2 * 60 * 60));

    let service = Service {
        client: reqwest::Client::new(),
        directory: root.join("images"),
        video_directory: videos,
        frames_directory: root.join("frames"),
        permits: tokio::sync::Semaphore::new(6),
        video_permits: tokio::sync::Semaphore::new(2),
        initialized: tokio::sync::OnceCell::new(),
        image_written: AtomicU64::new(0),
        video_written: AtomicU64::new(0),
        frames_written: AtomicU64::new(0),
        pruning: Arc::new(AtomicBool::new(false)),
    };
    let loaded = service.load(&video, 64).await.unwrap();
    assert_eq!(&loaded, &path);
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert!(
        modified
            .elapsed()
            .is_ok_and(|age| age < Duration::from_secs(5))
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn prune_skips_unremovable_and_recent_files_and_continues() {
    let directory = temp_directory("skip");
    std::fs::create_dir_all(&directory).unwrap();
    let locked = directory.join("locked.img");
    let removable = directory.join("removable.img");
    let recent = directory.join("recent.img");
    for path in [&locked, &removable, &recent] {
        std::fs::write(path, b"12345").unwrap();
    }
    set_age(&locked, Duration::from_secs(3 * 60 * 60));
    set_age(&removable, Duration::from_secs(2 * 60 * 60));
    set_age(&recent, Duration::from_secs(30));

    prune_with(&directory, 0, |path| {
        if path == locked.as_path() {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "in use",
            ))
        } else {
            std::fs::remove_file(path)
        }
    })
    .unwrap();

    assert!(locked.exists());
    assert!(!removable.exists());
    assert!(recent.exists());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn lru_touch_protects_a_cache_hit_from_pruning() {
    let directory = temp_directory("lru");
    std::fs::create_dir_all(&directory).unwrap();
    let touched = directory.join("touched.img");
    let untouched = directory.join("untouched.img");
    std::fs::write(&touched, b"12345").unwrap();
    std::fs::write(&untouched, b"12345").unwrap();
    set_age(&touched, Duration::from_secs(2 * 60 * 60));
    set_age(&untouched, Duration::from_secs(60 * 60));

    let now = SystemTime::now();
    assert!(touch_due(now - Duration::from_secs(2 * 60 * 60), now));
    assert!(!touch_due(now - Duration::from_secs(30 * 60), now));
    touch_modified(&touched).unwrap();

    prune(&directory, 8).unwrap();

    assert!(touched.exists());
    assert!(!untouched.exists());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn profile_cosmetic_urls() {
    let url = |spec: ImageSpec, size| image_url(&spec, size).map(String::from);
    let decoration = |asset: &str, animated| ImageSpec::AvatarDecoration {
        asset: asset.into(),
        animated,
    };
    assert_eq!(
        url(decoration("a_f0000000000000000000000000000016", true), 96).as_deref(),
        Some(
            "https://cdn.discordapp.com/avatar-decoration-presets/a_f0000000000000000000000000000016.png?size=128&passthrough=true"
        )
    );
    assert_eq!(
        url(decoration("a_f0000000000000000000000000000016", false), 96).as_deref(),
        Some(
            "https://cdn.discordapp.com/avatar-decoration-presets/a_f0000000000000000000000000000016.png?size=128&passthrough=false"
        )
    );
    assert_eq!(
        url(decoration("f000000000000015", true), 64).as_deref(),
        Some(
            "https://cdn.discordapp.com/avatar-decoration-presets/f000000000000015.png?size=64&passthrough=false"
        )
    );
    let nameplate = "nameplates/sample_plate_one/1191169245577216079/";
    for (animated, file) in [(false, "static"), (true, "img")] {
        let spec = ImageSpec::Nameplate {
            asset: nameplate.into(),
            animated,
        };
        assert_eq!(
            url(spec, 0),
            Some(format!(
                "https://cdn.discordapp.com/assets/collectibles/nameplates/sample_plate_one/1191169245577216079/{file}.png"
            ))
        );
    }
    assert_eq!(
        url(
            ImageSpec::Banner {
                user_id: 1,
                hash: "a_ff".into()
            },
            600
        )
        .as_deref(),
        Some("https://cdn.discordapp.com/banners/1/a_ff.png?size=1024")
    );
    assert_eq!(
        url(
            ImageSpec::GuildMemberBanner {
                guild_id: 2,
                user_id: 1,
                hash: "ff".into()
            },
            300
        )
        .as_deref(),
        Some("https://cdn.discordapp.com/guilds/2/users/1/banners/ff.png?size=512")
    );
    assert_eq!(
        url(
            ImageSpec::GuildBanner {
                guild_id: 2,
                hash: "ff".into()
            },
            300
        )
        .as_deref(),
        Some("https://cdn.discordapp.com/banners/2/ff.webp?size=512")
    );
    let guild_icon = ImageSpec::GuildIcon {
        guild_id: 2,
        hash: "a_ff".into(),
    };
    assert_eq!(
        url(guild_icon.animated().unwrap(), 48).as_deref(),
        Some("https://cdn.discordapp.com/icons/2/a_ff.webp?size=64&animated=true")
    );
    let guild_banner = ImageSpec::GuildBanner {
        guild_id: 2,
        hash: "a_ff".into(),
    };
    assert_eq!(
        url(guild_banner.animated().unwrap(), 300).as_deref(),
        Some("https://cdn.discordapp.com/banners/2/a_ff.webp?size=512&animated=true")
    );
    assert!(
        ImageSpec::GuildBanner {
            guild_id: 2,
            hash: "not/a/hash".into()
        }
        .animated()
        .is_none()
    );
    assert_eq!(
        url(
            ImageSpec::GuildSplash {
                guild_id: 2,
                hash: "ff".into()
            },
            512
        )
        .as_deref(),
        Some("https://cdn.discordapp.com/splashes/2/ff.png?size=512")
    );
    assert_eq!(
        url(
            ImageSpec::Badge {
                icon: "f0000000000000000000000000000001".into()
            },
            16
        )
        .as_deref(),
        Some("https://cdn.discordapp.com/badge-icons/f0000000000000000000000000000001.png")
    );
    let frame = profile_frame_layer_url(1191169253965824081, 1191169279131648087);
    assert_eq!(
        frame,
        "https://cdn.discordapp.com/media/v1/collectibles-shop/1191169253965824081/1191169279131648087/static"
    );
    assert_eq!(
        url(ImageSpec::Collectible { url: frame.clone() }, 0),
        Some(frame)
    );
    let banner = ImageSpec::Banner {
        user_id: 1,
        hash: "a_ff".into(),
    };
    assert_eq!(
        url(banner.animated().unwrap(), 480).as_deref(),
        Some("https://cdn.discordapp.com/banners/1/a_ff.webp?size=512&animated=true")
    );
    let avatar = ImageSpec::GuildAvatar {
        guild_id: 2,
        user_id: 1,
        hash: "a_ee".into(),
    };
    assert_eq!(
        url(avatar.animated().unwrap(), 80).as_deref(),
        Some(
            "https://cdn.discordapp.com/guilds/2/users/1/avatars/a_ee.webp?size=128&animated=true"
        )
    );
    let still = ImageSpec::Avatar {
        user_id: 1,
        hash: "ff".into(),
    };
    assert!(still.animated().is_none());
    assert!(
        image_url(
            &ImageSpec::Animated {
                still: Box::new(still)
            },
            80
        )
        .is_none()
    );
    let effect = "https://cdn.discordapp.com/assets/content/f0000000000000000000000000000000000000000000002b";
    assert_eq!(
        url(ImageSpec::Collectible { url: effect.into() }, 0).as_deref(),
        Some(effect)
    );
}

#[test]
fn profile_cosmetic_urls_reject_untrusted_input() {
    for asset in ["", "a_../x", "a_x?size=1", "a_x#y", "a/b", &"a".repeat(129)] {
        let spec = ImageSpec::AvatarDecoration {
            asset: asset.into(),
            animated: true,
        };
        assert!(image_url(&spec, 96).is_none(), "{asset}");
        assert!(
            image_url(&ImageSpec::Badge { icon: asset.into() }, 16).is_none(),
            "{asset}"
        );
    }
    for asset in [
        "nameplates/../x/",
        "nameplates/a/b",
        "harvest/a/",
        "nameplates//a/",
        "nameplates/a?b/",
        "nameplates/a.b/",
    ] {
        for animated in [false, true] {
            let spec = ImageSpec::Nameplate {
                asset: asset.into(),
                animated,
            };
            assert!(image_url(&spec, 0).is_none(), "{asset}");
        }
    }
    for url in [
        "http://cdn.discordapp.com/media/v1/collectibles-shop/a",
        "https://media.discordapp.net/media/v1/collectibles-shop/a",
        "https://cdn.discordapp.com/attachments/1/2/a.png",
        "https://cdn.discordapp.com/media/v1/collectibles-shop/../../attachments/a",
        "https://evil.example/media/v1/collectibles-shop/a",
        "https://cdn.discordapp.com.evil.example/media/v1/collectibles-shop/a",
    ] {
        let spec = ImageSpec::Collectible { url: url.into() };
        assert!(image_url(&spec, 0).is_none(), "{url}");
    }
}

#[test]
fn sticker_urls_follow_format_and_size_buckets() {
    let url =
        |format, size| image_url(&ImageSpec::Sticker { id: 123, format }, size).map(String::from);
    assert_eq!(
        url(1, 160).as_deref(),
        Some("https://media.discordapp.net/stickers/123.png?size=256")
    );
    assert_eq!(
        url(2, 160).as_deref(),
        Some("https://media.discordapp.net/stickers/123.png?size=256")
    );
    assert_eq!(
        url(4, 160).as_deref(),
        Some("https://media.discordapp.net/stickers/123.gif?size=256")
    );
    assert!(url(3, 160).is_none());
    assert!(url(0, 160).is_none());
    assert!(url(5, 160).is_none());
}

#[test]
fn video_urls_preserve_parameters_and_reject_untrusted_sources() {
    for host in [
        "cdn.discordapp.com",
        "media.discordapp.net",
        "images-ext-1.discordapp.net",
        "images-ext-123.discordapp.net",
    ] {
        for dimensions in [(0, 0), (400, 200)] {
            let raw = format!(
                "https://{host}/external/x.mp4?width=1&height=2&size=3&format=mp4&token=a%2Fb#fragment"
            );
            let spec = ImageSpec::Video {
                url: raw.clone(),
                width: dimensions.0,
                height: dimensions.1,
            };
            for size in [0, 64, 4096] {
                assert_eq!(
                    image_url(&spec, size).unwrap().as_str(),
                    raw.split('#').next().unwrap()
                );
            }
            assert!(file_name(&spec, 64).ends_with("-64.img"));
        }
    }
    for url in [
        "https://media.tenor.com.evil.test/a.mp4",
        "https://i.giphy.com.evil.test/a.mp4",
        "http://cdn.discordapp.com/a.mp4",
        "https://user:pass@cdn.discordapp.com/a.mp4",
        "https://media.discordapp.net:444/a.mp4",
        "https://images-ext-.discordapp.net/a.mp4",
        "https://images-ext-x.discordapp.net/a.mp4",
        "https://images-ext-1.discordapp.net.evil.test/a.mp4",
        "not a URL",
    ] {
        assert!(
            image_url(
                &ImageSpec::Video {
                    url: url.into(),
                    width: 1,
                    height: 1,
                },
                64
            )
            .is_none()
        );
    }
}

#[test]
fn video_posters_use_the_proxy_webp_format() {
    let raw = "https://media.discordapp.net/attachments/1/2/clip.mp4?ex=abc&width=9&height=9&size=9&format=mp4#fragment";
    let spec = ImageSpec::VideoPoster {
        id: 1,
        url: raw.into(),
        width: 640,
        height: 360,
    };
    assert_eq!(
        image_url(&spec, 128).unwrap().as_str(),
        "https://media.discordapp.net/attachments/1/2/clip.mp4?ex=abc&width=128&height=72&format=webp"
    );
    assert!(
        image_url(
            &ImageSpec::VideoPoster {
                id: 1,
                url: raw.into(),
                width: 0,
                height: 360,
            },
            128,
        )
        .is_none()
    );
    assert_eq!(
        trusted_video_url(raw).as_deref(),
        Some(
            "https://media.discordapp.net/attachments/1/2/clip.mp4?ex=abc&width=9&height=9&size=9&format=mp4"
        )
    );
    let redirect = "https://images-ext-1.discordapp.net/external/abc/%3Furl%3Dhttps%253A%252F%252Fvideo.twimg.com%252Fa.mp4/https/api.fxtwitter.com/2/go";
    assert_eq!(trusted_video_url(redirect).as_deref(), Some(redirect));
    for url in [
        "https://media.tenor.com.evil.test/a.mp4",
        "https://cdn.discordapp.com/a.m3u8",
        "https://media.discordapp.net/external/abc/https/example.com/go",
        "http://cdn.discordapp.com/a.mp4",
        "https://cdn.discordapp.com:444/a.mp4",
    ] {
        assert!(trusted_video_url(url).is_none(), "{url}");
    }
}

#[test]
fn video_and_image_download_formats_are_separate() {
    let video = ImageSpec::Video {
        url: String::new(),
        width: 1,
        height: 1,
    };
    let image = ImageSpec::DefaultAvatar { index: 0 };
    for bytes in [b"\0\0\0\x18ftypisom".as_slice(), b"\0\0\0\x20ftypmp42"] {
        assert!(download_bytes(&video, bytes));
        assert!(!download_bytes(&image, bytes));
    }
    for bytes in [
        b"".as_slice(),
        b"ftyp",
        b"\0\0\0\x18fty",
        b"\0\0\0\x18moov",
        b"<html>",
    ] {
        assert!(!download_bytes(&video, bytes));
        assert!(!download_bytes(&image, bytes));
    }
    for bytes in [
        b"\x89PNG\r\n\x1a\n".as_slice(),
        &[0xff, 0xd8, 0xff],
        b"GIF87a",
        b"GIF89a",
        b"RIFF\0\0\0\0WEBP",
    ] {
        assert!(download_bytes(&image, bytes));
        assert!(!download_bytes(&video, bytes));
    }
}

#[test]
fn avif_images_use_proxy_webp_with_explicit_animation_and_preserve_signatures() {
    for host in [
        "images-ext-1.discordapp.net",
        "images-ext-123.discordapp.net",
        "media.discordapp.net",
        "cdn.discordapp.com",
    ] {
        let path = if host == "cdn.discordapp.com" {
            "/attachments/1/2/image.AVIF"
        } else {
            "/external/signature/%3Furl%3Dhttps%3A%2F%2Fexample.com%2Fclip.mp4/https/example.com/convert.AVIF"
        };
        let raw = format!(
            "https://{host}{path}?ex=expires&is=issued&hm=signature&token=a%2Fb%2Bc&quality=lossless&width=1&height=2&size=3&format=avif&format=avif&animated=false&animated=false#fragment"
        );
        for animated in [false, true] {
            for (width, height) in [(914, 1280), (0, 0), (914, 0), (0, 1280)] {
                let spec = ImageSpec::Media {
                    url: raw.clone(),
                    width,
                    height,
                    animated,
                };
                let url = image_url(&spec, 500).unwrap();
                let expected_host = if host == "cdn.discordapp.com" {
                    "media.discordapp.net"
                } else {
                    host
                };
                assert_eq!(url.host_str(), Some(expected_host));
                assert_eq!(url.path(), path);
                assert!(url.fragment().is_none());
                let sizing = if width != 0 && height != 0 {
                    "&width=365&height=512"
                } else {
                    ""
                };
                assert_eq!(
                    url.query().unwrap(),
                    format!(
                        "ex=expires&is=issued&hm=signature&token=a%2Fb%2Bc&quality=lossless{sizing}&format=webp&animated={animated}"
                    )
                );
                assert!(matches!(&spec, ImageSpec::Media { url, .. } if url == &raw));
                assert_eq!(still_spec(&spec), !animated);
            }
        }
    }
}

#[test]
fn avif_attachments_preserve_animation_and_other_formats_keep_their_urls() {
    let attachment = ImageSpec::Attachment {
        id: 1,
        url: "https://media.discordapp.net/attachments/1/2/image.avif?ex=signature".into(),
        width: 400,
        height: 200,
    };
    assert_eq!(
        image_url(&attachment, 100).unwrap().as_str(),
        "https://media.discordapp.net/attachments/1/2/image.avif?ex=signature&width=128&height=64&format=webp&animated=true"
    );
    assert!(!still_spec(&attachment));

    for extension in ["gif", "webp", "png", "jpg", "avif.png"] {
        let raw = format!(
            "https://images-ext-1.discordapp.net/external/image.{extension}?token=keep&format={extension}"
        );
        let spec = ImageSpec::Media {
            url: raw.clone(),
            width: 400,
            height: 200,
            animated: true,
        };
        assert_eq!(
            image_url(&spec, 100).unwrap().as_str(),
            format!("{raw}&width=128&height=64&animated=true")
        );
    }
    let raw = "https://media.discordapp.net/external/image.png?source=image.avif";
    let spec = ImageSpec::Media {
        url: raw.into(),
        width: 0,
        height: 0,
        animated: false,
    };
    assert_eq!(image_url(&spec, 64).unwrap().as_str(), raw);
    let video = ImageSpec::Video {
        url: "https://media.discordapp.net/attachments/1/2/image.avif?format=avif&width=1".into(),
        width: 400,
        height: 200,
    };
    assert_eq!(
        image_url(&video, 100).unwrap().as_str(),
        "https://media.discordapp.net/attachments/1/2/image.avif?format=avif&width=1"
    );
}

#[test]
fn avif_conversion_bypasses_legacy_download_and_frame_cache_names_only() {
    fn legacy_name(spec: &ImageSpec, size: u32) -> String {
        let mut hasher = DefaultHasher::new();
        spec.hash(&mut hasher);
        let suffix = if still_spec(spec) { "-still" } else { "" };
        format!(
            "{:016x}{suffix}-{}.img",
            hasher.finish(),
            rounded_size(size)
        )
    }

    let service = Service {
        client: reqwest::Client::new(),
        directory: "images".into(),
        video_directory: "videos".into(),
        frames_directory: "frames".into(),
        permits: Semaphore::new(6),
        video_permits: Semaphore::new(2),
        initialized: OnceCell::new(),
        image_written: AtomicU64::new(0),
        video_written: AtomicU64::new(0),
        frames_written: AtomicU64::new(0),
        pruning: Arc::new(AtomicBool::new(false)),
    };
    for animated in [false, true] {
        let spec = ImageSpec::Media {
            url: "https://images-ext-1.discordapp.net/external/image.avif".into(),
            width: 400,
            height: 200,
            animated,
        };
        assert_ne!(file_name(&spec, 100), legacy_name(&spec, 100));
        assert_eq!(file_name(&spec, 100), file_name(&spec, 128));
        assert_eq!(
            service.frames_path(&spec, 100),
            Path::new("frames").join(file_name(&spec, 100))
        );
        assert_ne!(
            service.frames_path(&spec, 100),
            Path::new("frames").join(legacy_name(&spec, 100))
        );
    }
    let attachment = ImageSpec::Attachment {
        id: 1,
        url: "https://cdn.discordapp.com/attachments/1/2/image.avif".into(),
        width: 400,
        height: 200,
    };
    assert_ne!(file_name(&attachment, 100), legacy_name(&attachment, 100));
    for spec in [
        ImageSpec::Media {
            url: "https://media.discordapp.net/external/image.gif".into(),
            width: 400,
            height: 200,
            animated: true,
        },
        ImageSpec::Video {
            url: "https://media.discordapp.net/external/image.avif".into(),
            width: 400,
            height: 200,
        },
        ImageSpec::VideoPoster {
            id: 1,
            url: "https://media.discordapp.net/external/image.avif".into(),
            width: 400,
            height: 200,
        },
    ] {
        assert_eq!(file_name(&spec, 100), legacy_name(&spec, 100));
    }
}

#[test]
fn external_proxy_hosts_and_unknown_media_dimensions() {
    for host in [
        "images-ext-1.discordapp.net",
        "images-ext-2.discordapp.net",
        "images-ext-123.discordapp.net",
    ] {
        let url = format!(
            "https://{host}/external/image.png?token=keep&width=1&height=2&size=3#fragment"
        );
        for (width, height) in [(0, 0), (0, 200), (100, 0)] {
            let spec = ImageSpec::Media {
                url: url.clone(),
                width,
                height,
                animated: false,
            };
            assert_eq!(
                image_url(&spec, 64).unwrap().as_str(),
                format!("https://{host}/external/image.png?token=keep")
            );
            assert!(
                image_url(
                    &ImageSpec::Attachment {
                        id: 1,
                        url: url.clone(),
                        width,
                        height
                    },
                    64
                )
                .is_none()
            );
        }
        let spec = ImageSpec::Media {
            url: url.clone(),
            width: 400,
            height: 200,
            animated: false,
        };
        assert_eq!(
            image_url(&spec, 100).unwrap().as_str(),
            format!("https://{host}/external/image.png?token=keep&width=128&height=64")
        );
        let animated = ImageSpec::Media {
            url,
            width: 400,
            height: 200,
            animated: true,
        };
        assert_eq!(
            image_url(&animated, 100).unwrap().as_str(),
            format!(
                "https://{host}/external/image.png?token=keep&width=128&height=64&animated=true"
            )
        );
    }
    for host in [
        "images-ext-.discordapp.net",
        "images-ext-x.discordapp.net",
        "images-ext-1.discordapp.net.evil.test",
        "images-ext-1.evil.test",
        "images-ext-1x.discordapp.net",
    ] {
        assert!(!allowed_url(
            &reqwest::Url::parse(&format!("https://{host}/a")).unwrap()
        ));
    }
    assert_eq!(
        image_url(
            &ImageSpec::Media {
                url: "https://images-ext-1.discordapp.net/a".into(),
                width: 0,
                height: 0,
                animated: false,
            },
            64
        )
        .unwrap()
        .as_str(),
        "https://images-ext-1.discordapp.net/a"
    );
}

#[test]
fn app_images_request_a_sized_copy() {
    let url = |uploaded| {
        image_url(
            &ImageSpec::AppImage {
                application_id: 7,
                hash: "abc123".into(),
                uploaded,
                animated: false,
            },
            300,
        )
        .unwrap()
        .to_string()
    };
    assert_eq!(
        url(false),
        "https://cdn.discordapp.com/app-icons/7/abc123.png?size=512"
    );
    assert_eq!(
        url(true),
        "https://cdn.discordapp.com/app-assets/7/abc123.webp?size=512"
    );
    let still = ImageSpec::AppImage {
        application_id: 7,
        hash: "abc123".into(),
        uploaded: true,
        animated: true,
    };
    assert_eq!(
        image_url(&still.animated().unwrap(), 300)
            .unwrap()
            .to_string(),
        "https://cdn.discordapp.com/app-assets/7/abc123.webp?size=512&animated=true"
    );
    let cover = ImageSpec::AppCover {
        application_id: 7,
        hash: "abc123".into(),
    };
    assert_eq!(
        image_url(&cover, 300).unwrap().to_string(),
        "https://cdn.discordapp.com/app-icons/7/abc123.png?size=512&keep_aspect_ratio=true"
    );
}

#[test]
fn spotify_album_art_loads_as_an_image_only() {
    let url = "https://i.scdn.co/image/synthetic-album-art";
    assert_eq!(
        image_url(
            &ImageSpec::Media {
                url: url.into(),
                width: 300,
                height: 300,
                animated: false,
            },
            128
        )
        .unwrap()
        .as_str(),
        "https://i.scdn.co/image/synthetic-album-art?width=128&height=128"
    );
    assert!(trusted_video_url("https://i.scdn.co/image/synthetic.mp4").is_none());
    assert!(!allowed_url(&reqwest::Url::parse(url).unwrap()));
    for url in [
        "http://i.scdn.co/image/synthetic-album-art",
        "https://i.scdn.co.evil.test/image/synthetic-album-art",
        "https://i.scdn.co:8443/image/synthetic-album-art",
        "https://user:pass@i.scdn.co/image/synthetic-album-art",
    ] {
        assert!(
            image_url(
                &ImageSpec::Media {
                    url: url.into(),
                    width: 0,
                    height: 0,
                    animated: false,
                },
                64
            )
            .is_none(),
            "{url}"
        );
    }
}

#[test]
fn gif_picker_media_loads_from_provider_hosts_only() {
    for host in [
        "static.klipy.com",
        "media.tenor.com",
        "media.giphy.com",
        "i.giphy.com",
        "media0.giphy.com",
        "media1.giphy.com",
        "media2.giphy.com",
        "media3.giphy.com",
        "media4.giphy.com",
    ] {
        let url = format!("https://{host}/gif.webp");
        assert_eq!(
            image_url(
                &ImageSpec::Media {
                    url: url.clone(),
                    width: 200,
                    height: 200,
                    animated: true,
                },
                128
            )
            .unwrap()
            .as_str(),
            format!("https://{host}/gif.webp?width=128&height=128&animated=true")
        );
        assert!(!allowed_url(&reqwest::Url::parse(&url).unwrap()), "{url}");
    }
    for url in [
        "https://media.tenor.com/clip.mp4",
        "https://media4.giphy.com/clip.mp4",
    ] {
        assert!(
            image_url(
                &ImageSpec::Video {
                    url: url.into(),
                    width: 200,
                    height: 200,
                },
                64
            )
            .is_some(),
            "{url}"
        );
    }
    for url in [
        "http://static.klipy.com/gif.webp",
        "https://static.klipy.com.evil.test/gif.webp",
        "https://media.tenor.com.evil.test/gif.webp",
        "https://media1.giphy.com.evil.test/gif.webp",
        "https://static.klipy.com:8443/gif.webp",
        "https://user:pass@static.klipy.com/gif.webp",
        "https://media.gfycat.com/gif.webp",
    ] {
        assert!(
            image_url(
                &ImageSpec::Media {
                    url: url.into(),
                    width: 0,
                    height: 0,
                    animated: false,
                },
                64
            )
            .is_none(),
            "{url}"
        );
    }
}

#[test]
fn gif_provider_videos_are_trusted_only_for_mp4_and_webm() {
    for (url, trusted) in [
        (
            "https://media.tenor.com/abc/clip.mp4",
            "https://media.tenor.com/abc/clip.mp4",
        ),
        (
            "https://media.tenor.com/abc/clip.webm",
            "https://media.tenor.com/abc/clip.webm",
        ),
        (
            "https://media.tenor.com/abc/clip.mp4?ex=1&size=2#fragment",
            "https://media.tenor.com/abc/clip.mp4?ex=1&size=2",
        ),
        (
            "https://static.klipy.com/ii/aa/bb/mp4/1.mp4",
            "https://static.klipy.com/ii/aa/bb/mp4/1.mp4",
        ),
        (
            "https://media1.giphy.com/media/x/clip.MP4",
            "https://media1.giphy.com/media/x/clip.MP4",
        ),
        (
            "https://media.giphy.com/media/x/clip.webm",
            "https://media.giphy.com/media/x/clip.webm",
        ),
        (
            "https://i.giphy.com/clip.mp4",
            "https://i.giphy.com/clip.mp4",
        ),
    ] {
        assert_eq!(trusted_video_url(url).as_deref(), Some(trusted), "{url}");
    }
    for url in [
        "http://media.tenor.com/clip.mp4",
        "https://media.tenor.com.evil.test/clip.mp4",
        "https://media.tenor.com:8443/clip.mp4",
        "https://user:pass@media.tenor.com/clip.mp4",
        "https://static.klipy.com/clip.mov",
        "https://static.klipy.com/clip.m4v",
        "https://static.klipy.com/clip.gif",
        "https://static.klipy.com/clip",
        "https://static.klipy.com/clip?file=1.mp4",
        "https://media.gfycat.com/clip.mp4",
    ] {
        assert!(trusted_video_url(url).is_none(), "{url}");
    }
}

#[test]
fn cdn_urls_and_size_buckets() {
    assert_eq!(
        image_url(
            &ImageSpec::GuildTagBadge {
                guild_id: 9,
                hash: "badge".into()
            },
            20
        )
        .unwrap()
        .as_str(),
        "https://cdn.discordapp.com/guild-tag-badges/9/badge.png?size=32"
    );
    let avatar = ImageSpec::GuildAvatar {
        guild_id: 1,
        user_id: 2,
        hash: "a_hash".into(),
    };
    assert_eq!(
        image_url(&avatar, 32).unwrap().as_str(),
        "https://cdn.discordapp.com/guilds/1/users/2/avatars/a_hash.png?size=32"
    );
    for animated in [true, false] {
        assert_eq!(
            image_url(&ImageSpec::Emoji { id: 123, animated }, 100)
                .unwrap()
                .as_str(),
            format!("https://cdn.discordapp.com/emojis/123.webp?size=128&animated={animated}")
        );
    }
    assert_eq!(
        [0, 16, 17, 32, 100, 4097, u32::MAX].map(rounded_size),
        [16, 16, 32, 32, 128, 4096, 4096]
    );
    let icon = ImageSpec::GuildIcon {
        guild_id: 1,
        hash: "a_hash".into(),
    };
    assert_eq!(
        image_url(&icon, 100).unwrap().as_str(),
        "https://cdn.discordapp.com/icons/1/a_hash.png?size=128"
    );
    let avatar = ImageSpec::Avatar {
        user_id: 2,
        hash: "hash".into(),
    };
    assert_eq!(
        image_url(&avatar, 32).unwrap().as_str(),
        "https://cdn.discordapp.com/avatars/2/hash.png?size=32"
    );
    assert_eq!(
        image_url(&ImageSpec::DefaultAvatar { index: 5 }, 64)
            .unwrap()
            .as_str(),
        "https://cdn.discordapp.com/embed/avatars/5.png"
    );
    let attachment = ImageSpec::Attachment {
        id: 3,
        url: "https://media.discordapp.net/attachments/1/2/a.gif?ex=signature&width=1&height=1"
            .into(),
        width: 400,
        height: 200,
    };
    assert_eq!(
        image_url(&attachment, 100).unwrap().as_str(),
        "https://media.discordapp.net/attachments/1/2/a.gif?ex=signature&width=128&height=64"
    );
    assert!(
        image_url(&attachment, 4096)
            .unwrap()
            .as_str()
            .ends_with("width=400&height=200")
    );
}

#[test]
fn hostile_urls_and_cache_names() {
    for url in [
        "https://evil.example/x.png",
        "https://evil.example/image.avif",
        "http://media.discordapp.net/image.avif",
        "https://images-ext-1.discordapp.net.evil.test/image.avif",
        "https://user:pass@cdn.discordapp.com/image.avif",
        "https://media.discordapp.net:444/image.avif",
        "http://cdn.discordapp.com/a.png",
        "https://cdn.discordapp.com.evil.example/a",
        "https://user@cdn.discordapp.com/a",
        "https://media.discordapp.net:444/a",
    ] {
        let spec = ImageSpec::Attachment {
            id: 1,
            url: url.into(),
            width: 100,
            height: 100,
        };
        assert!(image_url(&spec, 64).is_none());
        let name = file_name(&spec, 64);
        assert!(!name.contains('?') && !name.contains('/') && !name.contains(".."));
        assert_eq!(name, file_name(&spec, 63));
    }
    assert!(
        image_url(
            &ImageSpec::Avatar {
                user_id: 1,
                hash: "../escape".into()
            },
            32
        )
        .is_none()
    );
}

#[test]
fn still_conversion_keeps_one_downscaled_frame() {
    let gif = {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        100,
                        50,
                        image::Rgba(color),
                    )))
                    .unwrap();
            }
        }
        bytes
    };
    let png = first_frame_png(&gif, 40).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let decoder = image::codecs::png::PngDecoder::new(Cursor::new(&png)).unwrap();
    assert!(!decoder.is_apng().unwrap());
    let decoded = image::load_from_memory(&png).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (40, 20));
}

#[test]
fn still_conversion_leaves_a_single_frame_png_alone() {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert!(first_frame_png(&png, 8).is_none());
}

#[test]
fn animated_specs_are_not_converted() {
    let still = ImageSpec::Media {
        url: "https://media.tenor.com/a.gif".into(),
        width: 100,
        height: 100,
        animated: false,
    };
    assert!(still_spec(&still));
    assert!(file_name(&still, 64).contains("-still-"));

    let animated_media = ImageSpec::Media {
        url: "https://media.tenor.com/a.gif".into(),
        width: 100,
        height: 100,
        animated: true,
    };
    assert!(!still_spec(&animated_media));
    assert!(!file_name(&animated_media, 64).contains("-still-"));

    for spec in [
        ImageSpec::Emoji {
            id: 1,
            animated: true,
        },
        ImageSpec::Sticker { id: 1, format: 2 },
        ImageSpec::Animated {
            still: Box::new(still.clone()),
        },
        ImageSpec::Video {
            url: "https://cdn.discordapp.com/a.mp4".into(),
            width: 1,
            height: 1,
        },
        ImageSpec::Collectible {
            url: "https://cdn.discordapp.com/a.png".into(),
        },
        ImageSpec::Attachment {
            id: 1,
            url: "https://cdn.discordapp.com/a.gif".into(),
            width: 1,
            height: 1,
        },
        ImageSpec::AvatarDecoration {
            asset: "a_decoration".into(),
            animated: true,
        },
        ImageSpec::Nameplate {
            asset: "nameplates/a/".into(),
            animated: true,
        },
        ImageSpec::AppImage {
            application_id: 1,
            hash: "asset".into(),
            uploaded: false,
            animated: true,
        },
    ] {
        assert!(!still_spec(&spec), "{spec:?}");
    }
}

#[test]
fn local_variants_have_no_remote_url() {
    for spec in [
        ImageSpec::LocalGif {
            path: PathBuf::from("/tmp/zcord-staged.gif"),
        },
        ImageSpec::LocalVideo {
            path: PathBuf::from("/tmp/zcord-staged.mp4"),
        },
    ] {
        assert!(image_url(&spec, 128).is_none(), "{spec:?}");
        assert!(spec.animated().is_none(), "{spec:?}");
    }
}

#[test]
fn local_paths_get_distinct_cache_names() {
    let first = ImageSpec::LocalGif {
        path: PathBuf::from("/tmp/zcord-a.gif"),
    };
    let second = ImageSpec::LocalGif {
        path: PathBuf::from("/tmp/zcord-b.gif"),
    };
    assert_ne!(file_name(&first, 128), file_name(&second, 128));
}

#[tokio::test]
async fn local_files_load_without_network_or_cache_directories() {
    let root = temp_directory("local");
    std::fs::create_dir_all(&root).unwrap();
    let gif = root.join("staged.gif");
    std::fs::write(&gif, b"GIF89a").unwrap();
    let service = Service {
        client: reqwest::Client::new(),
        directory: root.join("images"),
        video_directory: root.join("videos"),
        frames_directory: root.join("frames"),
        permits: tokio::sync::Semaphore::new(6),
        video_permits: tokio::sync::Semaphore::new(2),
        initialized: tokio::sync::OnceCell::new(),
        image_written: AtomicU64::new(0),
        video_written: AtomicU64::new(0),
        frames_written: AtomicU64::new(0),
        pruning: Arc::new(AtomicBool::new(false)),
    };
    let local = ImageSpec::LocalGif { path: gif.clone() };
    assert_eq!(service.load(&local, 128).await, Some(gif.clone()));
    let missing = ImageSpec::LocalVideo {
        path: root.join("missing.mp4"),
    };
    assert_eq!(service.load(&missing, 128).await, None);
    assert!(!root.join("images").exists());
    assert!(!root.join("videos").exists());
    assert!(!root.join("frames").exists());
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn atomic_files_and_pruning() {
    let directory = temp_directory("atomic");
    tokio::fs::create_dir(&directory).await.unwrap();
    let path = directory.join("test.img");
    atomic_write(&path, b"image fixture").await.unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), b"image fixture");
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
    set_age(&path, Duration::from_secs(2 * 60));
    prune(&directory, 0).unwrap();
    assert!(!path.exists());
    tokio::fs::remove_dir(directory).await.unwrap();
}
