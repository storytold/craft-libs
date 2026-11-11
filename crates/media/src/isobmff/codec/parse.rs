//! Sample entry parsing: `stsd` entry payloads, `esds`, `colr` and `btrt`.

use super::*;

// ---------------------------------------------------------------------------------------------
// Parsing

fn is_video_fourcc(f: &[u8; 4]) -> bool {
    matches!(
        f,
        b"avc1"
            | b"avc3"
            | b"hvc1"
            | b"hev1"
            | b"av01"
            | b"vp09"
            | b"vp08"
            | b"apch"
            | b"apcn"
            | b"apcs"
            | b"apco"
            | b"ap4h"
            | b"ap4x"
            | b"apv1"
            | b"jpeg"
            | b"mjpa"
            | b"mjpb"
            | b"AVdn"
            | b"AVdh"
            | b"mp4v"
            | b"encv"
            | b"raw "
            | b"2vuy"
            | b"v210"
    )
}

fn is_audio_fourcc(f: &[u8; 4]) -> bool {
    matches!(
        f,
        b"mp4a"
            | b"lpcm"
            | b"sowt"
            | b"twos"
            | b"in24"
            | b"in32"
            | b"fl32"
            | b"fl64"
            | b"ipcm"
            | b"fpcm"
            | b"alac"
            | b"Opus"
            | b"fLaC"
            | b"ac-3"
            | b"ec-3"
            | b".mp3"
            | b".mp2"
            | b"enca"
            | b"ulaw"
            | b"alaw"
            | b"NONE"
    )
}

/// Parse one `stsd` entry. `handler` is the track handler type; `is_qt` selects QuickTime
/// interpretation of the sound description version field.
pub(crate) fn parse_sample_entry(format: FourCc, payload: &[u8], handler: FourCc, is_qt: bool) -> Result<SampleEntry> {
    let mut c = Cur::labeled(payload, "sample entry");
    c.skip(6)?;
    let data_reference_index = c.u16()?;
    let f = &format.0;
    let unknown = || CodecConfig::Unknown { fourcc: format, raw: payload.to_vec() };
    if f == b"tmcd" || (handler == b"tmcd" && !is_video_fourcc(f)) {
        let mut tc = TimecodeConfig::default();
        c.skip(4)?;
        tc.flags = c.u32()?;
        tc.timescale = c.u32()?;
        tc.frame_duration = c.u32()?;
        tc.frames_per_second = c.u8()?;
        c.skip(1)?;
        for b in boxes(c.rest()).map_while(|b| b.ok()) {
            if b.kind == b"name" && b.payload.len() >= 4 {
                let n = u16::from_be_bytes([b.payload[0], b.payload[1]]) as usize;
                let s = b.payload.get(4..4 + n).unwrap_or(&b.payload[4..]);
                tc.name = Some(String::from_utf8_lossy(s).into_owned());
            }
        }
        return Ok(SampleEntry { format, data_reference_index, codec: CodecConfig::Timecode(tc), video: None, audio: None, bitrate: None });
    }
    // `raw ` names both uncompressed video and 8-bit unsigned PCM; the handler decides.
    let raw_audio = f == b"raw " && handler == b"soun";
    let video_like = (is_video_fourcc(f) && !raw_audio) || (handler == b"vide" && !is_audio_fourcc(f));
    let audio_like = is_audio_fourcc(f) || (handler == b"soun" && !video_like);
    if video_like {
        let mut v = VideoParams::default();
        let _version = c.u16()?;
        let _revision = c.u16()?;
        v.vendor = c.fourcc()?;
        v.temporal_quality = c.u32()?;
        v.spatial_quality = c.u32()?;
        v.width = c.u16()?;
        v.height = c.u16()?;
        v.horiz_resolution = c.u32()?;
        v.vert_resolution = c.u32()?;
        c.skip(4)?;
        v.frame_count = c.u16()?;
        let name = c.bytes(32)?;
        let n = (name[0] as usize).min(31);
        v.compressor_name = String::from_utf8_lossy(&name[1..1 + n]).into_owned();
        v.depth = c.u16()?;
        let _ctab = c.i16()?;
        let mut codec = None;
        let mut bitrate = None;
        let mut log_seen = false;
        for b in boxes(c.rest()).map_while(|b| b.ok()) {
            let p = b.payload;
            match &b.kind.0 {
                b"avcC" if matches!(f, b"avc1" | b"avc3") => codec = Some(CodecConfig::Avc(AvcConfig::parse(p)?)),
                b"hvcC" if matches!(f, b"hvc1" | b"hev1") => codec = Some(CodecConfig::Hevc(HevcConfig::parse(p)?)),
                b"av1C" if f == b"av01" => codec = Some(CodecConfig::Av1(Av1Config::parse(p)?)),
                b"vpcC" if f == b"vp09" => codec = Some(CodecConfig::Vp9(VpcConfig::parse(p)?)),
                b"apvC" if f == b"apv1" => codec = Some(CodecConfig::Apv(ApvConfig::parse(p)?)),
                b"colr" => v.color = parse_colr(p).ok(),
                b"logs" => {
                    // Two identifiers are ambiguous: neither is believed.
                    v.log_transfer_function =
                        if log_seen { None } else { std::str::from_utf8(p).ok().filter(|s| valid_log_identifier(s)).map(str::to_owned) };
                    log_seen = true;
                }
                b"mdcv" => v.mastering_display = MasteringDisplay::parse(p),
                b"clli" if p.len() >= 4 => v.content_light = Some((u16::from_be_bytes([p[0], p[1]]), u16::from_be_bytes([p[2], p[3]]))),
                b"pasp" => {
                    let mut c = Cur::labeled(p, "pasp");
                    v.pixel_aspect = Some((c.u32()?, c.u32()?));
                }
                b"clap" => {
                    let mut c = Cur::labeled(p, "clap");
                    v.clean_aperture = Some(CleanAperture {
                        width: (c.u32()?, c.u32()?),
                        height: (c.u32()?, c.u32()?),
                        horiz_offset: (c.i32()?, c.u32()?),
                        vert_offset: (c.i32()?, c.u32()?),
                    });
                }
                b"fiel" => {
                    let mut c = Cur::labeled(p, "fiel");
                    v.field_info = Some(FieldInfo { fields: c.u8()?, detail: c.u8()? });
                }
                b"gama" => v.gamma = Some(Cur::labeled(p, "gama").u32()?),
                b"btrt" => bitrate = Some(parse_btrt(p)?),
                _ => {}
            }
        }
        let codec = match codec {
            Some(c) => c,
            None => match f {
                b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" | b"ap4x" => CodecConfig::ProRes { fourcc: format },
                b"jpeg" | b"mjpa" | b"mjpb" => CodecConfig::Jpeg { fourcc: format },
                b"AVdn" | b"AVdh" => CodecConfig::Dnx { fourcc: format },
                b"apv1" => CodecConfig::Apv(ApvConfig::default()),
                _ => unknown(),
            },
        };
        return Ok(SampleEntry { format, data_reference_index, codec, video: Some(v), audio: None, bitrate });
    }
    if audio_like {
        let mut a = AudioParams { qt_version: c.u16()?, ..Default::default() };
        let _revision = c.u16()?;
        let _vendor = c.u32()?;
        a.channels = c.u16()? as u32;
        a.sample_size = c.u16()?;
        a.compression_id = c.i16()?;
        let _packet_size = c.u16()?;
        a.sample_rate = c.u32()? as f64 / 65536.0;
        if is_qt && a.qt_version == 1 {
            a.samples_per_packet = c.u32()?;
            a.bytes_per_packet = c.u32()?;
            a.bytes_per_frame = c.u32()?;
            a.bytes_per_sample = c.u32()?;
        } else if is_qt && a.qt_version == 2 {
            let _size_of_struct = c.u32()?;
            a.sample_rate = c.f64()?;
            a.channels = c.u32()?;
            if a.channels > u32::from(u16::MAX) {
                return Err(Error::Invalid(format!("QuickTime v2 sound entry with {} channels", a.channels)));
            }
            let _always = c.u32()?;
            a.const_bits_per_channel = c.u32()?;
            a.lpcm_flags = c.u32()?;
            a.const_bytes_per_packet = c.u32()?;
            a.const_frames_per_packet = c.u32()?;
        }
        // Flatten QuickTime `wave` children into the list of extension boxes.
        let mut ext: Vec<(FourCc, &[u8])> = Vec::new();
        for b in boxes(c.rest()).map_while(|b| b.ok()) {
            if b.kind == b"wave" {
                for w in boxes(b.payload).map_while(|b| b.ok()) {
                    ext.push((w.kind, w.payload));
                }
            } else {
                ext.push((b.kind, b.payload));
            }
        }
        let get = |k: &[u8; 4]| ext.iter().find(|(kk, _)| kk == k).map(|(_, p)| *p);
        if let Some(p) = get(b"enda") {
            let v = if p.len() >= 2 { u16::from_be_bytes([p[0], p[1]]) } else { p.first().copied().unwrap_or(0) as u16 };
            a.little_endian = Some(v != 0);
        }
        let bitrate = get(b"btrt").and_then(|p| parse_btrt(p).ok());
        let codec = match f {
            b"mp4a" => match get(b"esds") {
                Some(p) => parse_esds(p)?,
                None => unknown(),
            },
            b".mp3" | b".mp2" => CodecConfig::Mp3,
            b"alac" => match get(b"alac") {
                Some(p) if p.len() >= 4 => CodecConfig::Alac { cookie: p[4..].to_vec() },
                _ => unknown(),
            },
            b"Opus" => match get(b"dOps") {
                Some(p) => CodecConfig::Opus(OpusConfig::parse(p)?),
                None => unknown(),
            },
            b"fLaC" => match get(b"dfLa") {
                Some(p) => CodecConfig::Flac(FlacConfig::parse(p)?),
                None => unknown(),
            },
            b"ac-3" => CodecConfig::Ac3 { dac3: get(b"dac3").unwrap_or(&[]).to_vec() },
            b"ec-3" => CodecConfig::Eac3 { dec3: get(b"dec3").unwrap_or(&[]).to_vec() },
            b"lpcm" | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" | b"raw " | b"NONE" => CodecConfig::Pcm(qt_pcm(f, &a)?),
            b"ipcm" | b"fpcm" => {
                let mut pcm = PcmConfig {
                    bits: a.sample_size,
                    float: f == b"fpcm",
                    big_endian: true,
                    signed: true,
                    channels: a.channels,
                    sample_rate: a.sample_rate,
                };
                if let Some(p) = get(b"pcmC") {
                    let mut c = Cur::labeled(p, "pcmC");
                    c.full_header()?;
                    let flags = c.u8()?;
                    pcm.big_endian = flags & 1 == 0;
                    pcm.bits = c.u8()? as u16;
                }
                CodecConfig::Pcm(pcm)
            }
            _ => unknown(),
        };
        return Ok(SampleEntry { format, data_reference_index, codec, video: None, audio: Some(a), bitrate });
    }
    Ok(SampleEntry { format, data_reference_index, codec: unknown(), video: None, audio: None, bitrate: None })
}

fn qt_pcm(f: &[u8; 4], a: &AudioParams) -> Result<PcmConfig> {
    let mut p = PcmConfig { channels: a.channels, sample_rate: a.sample_rate, signed: true, ..Default::default() };
    match f {
        b"lpcm" => {
            p.bits = u16::try_from(a.const_bits_per_channel)
                .map_err(|_| Error::Invalid(format!("lpcm with {} bits per channel", a.const_bits_per_channel)))?;
            p.float = a.lpcm_flags & 1 != 0;
            p.big_endian = a.lpcm_flags & 2 != 0;
            p.signed = a.lpcm_flags & 4 != 0 || p.float;
            if p.bits == 0 {
                p.bits = a.sample_size;
            }
        }
        b"sowt" => {
            p.bits = if a.sample_size == 0 { 16 } else { a.sample_size };
        }
        b"twos" => {
            p.bits = if a.sample_size == 0 { 16 } else { a.sample_size };
            p.big_endian = true;
        }
        b"raw " | b"NONE" => {
            p.bits = if a.sample_size == 0 { 8 } else { a.sample_size };
            p.signed = p.bits != 8;
            p.big_endian = true;
        }
        b"in24" | b"in32" | b"fl32" | b"fl64" => {
            p.bits = match f {
                b"in24" => 24,
                b"in32" | b"fl32" => 32,
                _ => 64,
            };
            p.float = f[0] == b'f';
            p.big_endian = !a.little_endian.unwrap_or(false);
        }
        _ => {}
    }
    Ok(p)
}

fn parse_btrt(p: &[u8]) -> Result<BitRate> {
    let mut c = Cur::labeled(p, "btrt");
    Ok(BitRate { buffer_size: c.u32()?, max_bitrate: c.u32()?, avg_bitrate: c.u32()? })
}

fn parse_colr(p: &[u8]) -> Result<ColorInfo> {
    let mut c = Cur::labeled(p, "colr");
    let kind = c.fourcc()?;
    Ok(match &kind.0 {
        b"nclx" => {
            ColorInfo::Nclx { primaries: c.u16()?, transfer: c.u16()?, matrix: c.u16()?, full_range: c.u8().map(|v| v & 0x80 != 0).unwrap_or(false) }
        }
        b"nclc" => ColorInfo::Nclc { primaries: c.u16()?, transfer: c.u16()?, matrix: c.u16()? },
        _ => ColorInfo::Icc { kind, profile: c.rest().to_vec() },
    })
}

fn descriptor(c: &mut Cur) -> Result<(u8, usize)> {
    let tag = c.u8()?;
    let mut len = 0usize;
    for _ in 0..4 {
        let b = c.u8()?;
        len = (len << 7) | (b & 0x7F) as usize;
        if b & 0x80 == 0 {
            break;
        }
    }
    Ok((tag, len))
}

fn parse_esds(p: &[u8]) -> Result<CodecConfig> {
    let mut c = Cur::labeled(p, "esds");
    c.full_header()?;
    let (tag, len) = descriptor(&mut c)?;
    if tag != 3 {
        return Err(Error::Invalid(format!("esds: expected ES_Descriptor, got tag {tag}")));
    }
    let es = c.bytes(len.min(c.remaining()))?;
    let mut c = Cur::labeled(es, "ES_Descriptor");
    c.u16()?;
    let fl = c.u8()?;
    if fl & 0x80 != 0 {
        c.u16()?;
    }
    if fl & 0x40 != 0 {
        let n = c.u8()? as usize;
        c.skip(n)?;
    }
    if fl & 0x20 != 0 {
        c.u16()?;
    }
    while c.remaining() > 0 {
        let (tag, len) = descriptor(&mut c)?;
        let body = c.bytes(len.min(c.remaining()))?;
        if tag != 4 {
            continue;
        }
        let mut d = Cur::labeled(body, "DecoderConfigDescriptor");
        let oti = d.u8()?;
        let _stream_type = d.u8()?;
        let buffer_size = d.u24()?;
        let max_bitrate = d.u32()?;
        let avg_bitrate = d.u32()?;
        let mut asc = Vec::new();
        while d.remaining() > 0 {
            let (t, l) = descriptor(&mut d)?;
            let b = d.bytes(l.min(d.remaining()))?;
            if t == 5 {
                asc = b.to_vec();
                break;
            }
        }
        return Ok(match oti {
            0x69 | 0x6B => CodecConfig::Mp3,
            _ => {
                let mut a = AacConfig::from_asc(asc);
                a.max_bitrate = max_bitrate;
                a.avg_bitrate = avg_bitrate;
                a.buffer_size = buffer_size;
                CodecConfig::Aac(a)
            }
        });
    }
    Err(Error::Invalid("esds without DecoderConfigDescriptor".into()))
}
