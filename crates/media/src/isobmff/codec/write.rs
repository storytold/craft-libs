//! Sample entry serialization: `esds` and the visual and audio entry boxes.

use super::*;

fn put_descriptor(b: &mut BoxBuf, tag: u8, body: &[u8]) {
    b.u8(tag);
    let n = body.len() as u32;
    // Always use the 4-byte length form for simplicity (as many encoders do).
    b.u8(0x80 | ((n >> 21) & 0x7F) as u8);
    b.u8(0x80 | ((n >> 14) & 0x7F) as u8);
    b.u8(0x80 | ((n >> 7) & 0x7F) as u8);
    b.u8((n & 0x7F) as u8);
    b.bytes(body);
}

pub(crate) fn esds_payload(oti: u8, asc: &[u8], max_bitrate: u32, avg_bitrate: u32, buffer_size: u32) -> Vec<u8> {
    let mut dcd = BoxBuf::new();
    dcd.u8(oti);
    dcd.u8(0x15); // streamType audio (5) << 2 | upstream 0 | reserved 1
    dcd.u8((buffer_size >> 16) as u8);
    dcd.u16(buffer_size as u16);
    dcd.u32(max_bitrate);
    dcd.u32(avg_bitrate);
    if !asc.is_empty() {
        put_descriptor(&mut dcd, 5, asc);
    }
    let mut es = BoxBuf::new();
    es.u16(1); // ES_ID
    es.u8(0);
    put_descriptor(&mut es, 4, &dcd.buf);
    put_descriptor(&mut es, 6, &[2]); // SLConfigDescriptor, predefined = 2 (MP4)
    let mut b = BoxBuf::new();
    b.u32(0);
    put_descriptor(&mut b, 3, &es.buf);
    b.buf
}

impl SampleEntry {
    /// Convenience constructor for a video entry.
    pub fn video(format: FourCc, codec: CodecConfig, width: u16, height: u16) -> Self {
        let compressor_name = match &codec {
            CodecConfig::ProRes { fourcc } => match &fourcc.0 {
                b"apch" => "Apple ProRes 422 HQ",
                b"apcn" => "Apple ProRes 422",
                b"apcs" => "Apple ProRes 422 LT",
                b"apco" => "Apple ProRes 422 Proxy",
                b"ap4h" => "Apple ProRes 4444",
                _ => "Apple ProRes 4444 XQ",
            },
            CodecConfig::Jpeg { .. } => "Photo - JPEG",
            _ => "",
        };
        SampleEntry {
            format,
            data_reference_index: 1,
            codec,
            video: Some(VideoParams {
                width,
                height,
                horiz_resolution: 0x0048_0000,
                vert_resolution: 0x0048_0000,
                frame_count: 1,
                compressor_name: compressor_name.into(),
                depth: 24,
                ..Default::default()
            }),
            audio: None,
            bitrate: None,
        }
    }

    /// Convenience constructor for an audio entry (ISO v0 layout; the muxer upgrades PCM in MOV to v2).
    pub fn audio(format: FourCc, codec: CodecConfig, channels: u32, sample_rate: f64, sample_size: u16) -> Self {
        SampleEntry {
            format,
            data_reference_index: 1,
            codec,
            video: None,
            audio: Some(AudioParams { channels, sample_rate, sample_size, ..Default::default() }),
            bitrate: None,
        }
    }

    /// H.264 entry (`avc1`).
    pub fn avc(cfg: AvcConfig, width: u16, height: u16) -> Self {
        Self::video(FourCc(*b"avc1"), CodecConfig::Avc(cfg), width, height)
    }
    /// HEVC entry (`hvc1`, parameter sets in the config only).
    pub fn hevc(cfg: HevcConfig, width: u16, height: u16) -> Self {
        Self::video(FourCc(*b"hvc1"), CodecConfig::Hevc(cfg), width, height)
    }
    /// AV1 entry (`av01`).
    pub fn av1(cfg: Av1Config, width: u16, height: u16) -> Self {
        Self::video(FourCc(*b"av01"), CodecConfig::Av1(cfg), width, height)
    }
    /// ProRes entry; `fourcc` one of `apch`/`apcn`/`apcs`/`apco`/`ap4h`/`ap4x`.
    pub fn prores(fourcc: FourCc, width: u16, height: u16) -> Self {
        let mut e = Self::video(fourcc, CodecConfig::ProRes { fourcc }, width, height);
        if let Some(v) = e.video.as_mut()
            && matches!(&fourcc.0, b"ap4h" | b"ap4x")
        {
            v.depth = 32;
        }
        e
    }
    /// Avid DNxHD / DNxHR entry (`AVdn`, or `AVdh` for DNxHR).
    pub fn dnx(fourcc: FourCc, width: u16, height: u16) -> Self {
        let mut e = Self::video(fourcc, CodecConfig::Dnx { fourcc }, width, height);
        if let Some(v) = e.video.as_mut() {
            v.depth = 24;
        }
        e
    }
    /// Advanced Professional Video entry (`apv1` + `apvC`).
    pub fn apv(cfg: ApvConfig, width: u16, height: u16) -> Self {
        let mut e = Self::video(FourCc(*b"apv1"), CodecConfig::Apv(cfg), width, height);
        if let Some(v) = e.video.as_mut() {
            v.compressor_name = "APV".into();
        }
        e
    }
    /// Motion-JPEG entry (`jpeg`).
    pub fn jpeg(width: u16, height: u16) -> Self {
        let f = FourCc(*b"jpeg");
        Self::video(f, CodecConfig::Jpeg { fourcc: f }, width, height)
    }
    /// AAC entry (`mp4a` + `esds`) from an AudioSpecificConfig.
    pub fn aac(asc: Vec<u8>, channels: u32, sample_rate: u32) -> Self {
        Self::audio(FourCc(*b"mp4a"), CodecConfig::Aac(AacConfig::from_asc(asc)), channels, sample_rate as f64, 16)
    }
    /// FLAC entry (`fLaC` + `dfLa`) from the 34-byte STREAMINFO body: the only metadata block.
    pub fn flac(streaminfo: &[u8; 34]) -> Self {
        let mut blocks = vec![0x80, 0, 0, 34];
        blocks.extend_from_slice(streaminfo);
        let mut full = vec![0; 4];
        full.extend_from_slice(&blocks);
        let cfg = FlacConfig::parse(&full).unwrap_or(FlacConfig { metadata_blocks: blocks, ..Default::default() });
        let (channels, rate, bits) = (u32::from(cfg.channels), f64::from(cfg.sample_rate), u16::from(cfg.bits_per_sample));
        Self::audio(FourCc(*b"fLaC"), CodecConfig::Flac(cfg), channels, rate, bits)
    }
    /// PCM entry. The muxer picks the on-disk format (`sowt`/`twos`/`lpcm` for MOV, `ipcm`/`fpcm` for MP4).
    pub fn pcm(cfg: PcmConfig) -> Self {
        Self::audio(FourCc(*b"lpcm"), CodecConfig::Pcm(cfg), cfg.channels, cfg.sample_rate, cfg.bits)
    }

    /// Serialize as a complete sample entry box. For PCM the format is chosen from the brand.
    pub(crate) fn write(&self, b: &mut BoxBuf, qt: bool) -> Result<()> {
        // An identifier the reader would not accept is refused, not truncated.
        if self.video.as_ref().and_then(|v| v.log_transfer_function.as_deref()).is_some_and(|s| !valid_log_identifier(s)) {
            return Err(Error::Invalid("invalid log transfer-function identifier".into()));
        }
        if let CodecConfig::Unknown { fourcc, raw } = &self.codec {
            let m = b.start(&fourcc.0);
            b.bytes(raw);
            b.end(m);
            return Ok(());
        }
        if let CodecConfig::Timecode(tc) = &self.codec {
            let m = b.start(b"tmcd");
            b.zeros(6);
            b.u16(self.data_reference_index);
            b.u32(0);
            b.u32(tc.flags);
            b.u32(tc.timescale);
            b.u32(tc.frame_duration);
            b.u8(tc.frames_per_second);
            b.u8(0);
            if let Some(name) = &tc.name {
                let n = b.start(b"name");
                b.u16(name.len() as u16);
                b.u16(0);
                b.bytes(name.as_bytes());
                b.end(n);
            }
            b.end(m);
            return Ok(());
        }
        if let Some(v) = &self.video {
            self.write_video(b, v);
            return Ok(());
        }
        let a = self.audio.clone().ok_or_else(|| Error::Mux("sample entry has neither video nor audio params".into()))?;
        // Choose the on-disk format and sound description version.
        let (format, version, pcm) = match &self.codec {
            CodecConfig::Pcm(p) => {
                if qt {
                    // Version 0 stores the rate as 16.16 fixed point; `raw ` is unsigned and
                    // `twos`/`sowt` are signed. Everything else needs `lpcm` v2.
                    let v0_rate = p.sample_rate < 65536.0;
                    if v0_rate && p.bits == 16 && !p.float && p.signed {
                        (if p.big_endian { *b"twos" } else { *b"sowt" }, 0u16, Some(*p))
                    } else if v0_rate && p.bits == 8 && !p.float {
                        (if p.signed { *b"twos" } else { *b"raw " }, 0, Some(*p))
                    } else {
                        (*b"lpcm", 2, Some(*p))
                    }
                } else {
                    (if p.float { *b"fpcm" } else { *b"ipcm" }, 0, Some(*p))
                }
            }
            _ => (self.format.0, if qt { a.qt_version.min(2) } else { 0 }, None),
        };
        let m = b.start(&format);
        b.zeros(6);
        b.u16(self.data_reference_index);
        if version == 2 {
            let p = pcm.unwrap_or_default();
            b.u16(2);
            b.u16(0);
            b.u32(0);
            b.u16(3);
            b.u16(16);
            b.i16(-2);
            b.u16(0);
            b.u32(65536);
            b.u32(72);
            b.f64(p.sample_rate);
            b.u32(p.channels);
            b.u32(0x7F00_0000);
            b.u32(p.bits as u32);
            let flags = (p.float as u32) | ((p.big_endian as u32) << 1) | (((p.signed && !p.float) as u32) << 2) | 8;
            b.u32(flags);
            b.u32(p.bytes_per_frame());
            b.u32(1);
        } else {
            b.u16(version);
            b.u16(0);
            b.u32(0);
            b.u16(a.channels.min(u16::MAX as u32) as u16);
            b.u16(match pcm {
                Some(p) => p.bits,
                None => {
                    if a.sample_size == 0 {
                        16
                    } else {
                        a.sample_size
                    }
                }
            });
            b.i16(if version == 1 { a.compression_id } else { 0 });
            b.u16(0);
            let sr = if a.sample_rate < 65536.0 { (a.sample_rate * 65536.0).round() as u32 } else { 0 };
            b.u32(sr);
            if version == 1 {
                b.u32(a.samples_per_packet);
                b.u32(a.bytes_per_packet);
                b.u32(a.bytes_per_frame);
                b.u32(a.bytes_per_sample);
            }
        }
        match &self.codec {
            CodecConfig::Aac(c) => {
                b.leaf(b"esds", &esds_payload(0x40, &c.asc, c.max_bitrate, c.avg_bitrate, c.buffer_size));
            }
            CodecConfig::Mp3 if format == *b"mp4a" => {
                b.leaf(b"esds", &esds_payload(0x6B, &[], 0, 0, 0));
            }
            CodecConfig::Alac { cookie } => {
                let m = b.start_full(b"alac", 0, 0);
                b.bytes(cookie);
                b.end(m);
            }
            CodecConfig::Opus(o) => b.leaf(b"dOps", &o.to_bytes()),
            CodecConfig::Flac(fl) => {
                let m = b.start_full(b"dfLa", 0, 0);
                b.bytes(&fl.metadata_blocks);
                b.end(m);
            }
            CodecConfig::Ac3 { dac3 } => b.leaf(b"dac3", dac3),
            CodecConfig::Eac3 { dec3 } => b.leaf(b"dec3", dec3),
            CodecConfig::Pcm(p) if !qt => {
                let m = b.start_full(b"pcmC", 0, 0);
                b.u8(if p.big_endian { 0 } else { 1 });
                b.u8(p.bits as u8);
                b.end(m);
            }
            CodecConfig::Pcm(p) if p.channels == 6 => {
                // QuickTime channel layout: kAudioChannelLayoutTag_MPEG_5_1_A (L R C LFE Ls Rs)
                let m = b.start_full(b"chan", 0, 0);
                b.u32((121 << 16) | 6);
                b.u32(0);
                b.u32(0);
                b.end(m);
            }
            _ => {}
        }
        if let Some(br) = self.bitrate {
            write_btrt(b, br);
        }
        b.end(m);
        Ok(())
    }

    fn write_video(&self, b: &mut BoxBuf, v: &VideoParams) {
        let m = b.start(&self.format.0);
        b.zeros(6);
        b.u16(self.data_reference_index);
        b.u16(0);
        b.u16(0);
        b.u32(v.vendor.as_u32());
        b.u32(v.temporal_quality);
        b.u32(v.spatial_quality);
        b.u16(v.width);
        b.u16(v.height);
        b.u32(v.horiz_resolution);
        b.u32(v.vert_resolution);
        b.u32(0);
        b.u16(v.frame_count);
        let name = v.compressor_name.as_bytes();
        let n = name.len().min(31);
        let mut nb = [0u8; 32];
        nb[0] = n as u8;
        nb[1..1 + n].copy_from_slice(&name[..n]);
        b.bytes(&nb);
        b.u16(v.depth);
        b.i16(-1);
        write_video_codec_config(b, &self.codec);
        write_video_descriptive_boxes(b, v);
        if let Some(br) = self.bitrate {
            write_btrt(b, br);
        }
        b.end(m);
    }
}

fn write_video_codec_config(b: &mut BoxBuf, codec: &CodecConfig) {
    match codec {
        CodecConfig::Avc(config) => b.leaf(b"avcC", &config.to_bytes()),
        CodecConfig::Hevc(config) => b.leaf(b"hvcC", &config.to_bytes()),
        CodecConfig::Av1(config) => b.leaf(b"av1C", &config.to_bytes()),
        CodecConfig::Vp9(config) => b.leaf(b"vpcC", &config.to_bytes()),
        CodecConfig::Apv(config) => b.leaf(b"apvC", &config.to_bytes()),
        _ => {}
    }
}

fn write_video_descriptive_boxes(b: &mut BoxBuf, v: &VideoParams) {
    if let Some(gamma) = v.gamma {
        b.leaf(b"gama", &gamma.to_be_bytes());
    }
    if let Some(field_info) = v.field_info {
        b.leaf(b"fiel", &[field_info.fields, field_info.detail]);
    }
    if let Some(color) = &v.color {
        write_color_box(b, color);
    }
    if let Some(log) = &v.log_transfer_function {
        b.leaf(b"logs", log.as_bytes());
    }
    if let Some(display) = &v.mastering_display {
        b.leaf(b"mdcv", &display.to_bytes());
    }
    if let Some((cll, fall)) = v.content_light {
        let mut data = cll.to_be_bytes().to_vec();
        data.extend_from_slice(&fall.to_be_bytes());
        b.leaf(b"clli", &data);
    }
    write_pixel_aspect_box(b, v.pixel_aspect);
    write_clean_aperture_box(b, v.clean_aperture);
}

fn write_color_box(b: &mut BoxBuf, color: &ColorInfo) {
    let m = b.start(b"colr");
    match color {
        ColorInfo::Nclx { primaries, transfer, matrix, full_range } => {
            b.bytes(b"nclx");
            b.u16(*primaries);
            b.u16(*transfer);
            b.u16(*matrix);
            b.u8(if *full_range { 0x80 } else { 0 });
        }
        ColorInfo::Nclc { primaries, transfer, matrix } => {
            b.bytes(b"nclc");
            b.u16(*primaries);
            b.u16(*transfer);
            b.u16(*matrix);
        }
        ColorInfo::Icc { kind, profile } => {
            b.bytes(&kind.0);
            b.bytes(profile);
        }
    }
    b.end(m);
}

fn write_pixel_aspect_box(b: &mut BoxBuf, pixel_aspect: Option<(u32, u32)>) {
    if let Some((horizontal, vertical)) = pixel_aspect {
        let m = b.start(b"pasp");
        b.u32(horizontal);
        b.u32(vertical);
        b.end(m);
    }
}

fn write_clean_aperture_box(b: &mut BoxBuf, clean_aperture: Option<CleanAperture>) {
    if let Some(aperture) = clean_aperture {
        let m = b.start(b"clap");
        b.u32(aperture.width.0);
        b.u32(aperture.width.1);
        b.u32(aperture.height.0);
        b.u32(aperture.height.1);
        b.i32(aperture.horiz_offset.0);
        b.u32(aperture.horiz_offset.1);
        b.i32(aperture.vert_offset.0);
        b.u32(aperture.vert_offset.1);
        b.end(m);
    }
}

fn write_btrt(b: &mut BoxBuf, br: BitRate) {
    let m = b.start(b"btrt");
    b.u32(br.buffer_size);
    b.u32(br.max_bitrate);
    b.u32(br.avg_bitrate);
    b.end(m);
}
