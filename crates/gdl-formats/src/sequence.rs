//! Sequenced sounds in the DCS sound banks (`AUDIO/*.VBK`), including the title/select and
//! shop music, rendered the way the game's driver plays them.
//!
//! # The format
//!
//! A `.VBK` file is a Midway DCS "BANK" (magic `KNBV`, big-endian; type 1.006 made with
//! VAGBANK 1.007). Layout, as read by `BankReadHeader` / `dcsReadCalls` / `dcsReadVags` in
//! the decomp (`src/game/audio/dcs.c`):
//!
//! ```text
//! 0x00  "KNBV"
//! 0x04  u32  size in bytes of the call list
//! 0x08  u32  version (0x01070106)
//! 0x0C  u32  number of calls (sounds)
//! 0x10  u32  number of VAG samples
//! 0x14  call list: u16 words
//! ....  samples: 48-byte "VAGp" header (+0x04 version, +0x0C data length, +0x10 rate,
//!       +0x20 name); when the version is 0x28 a standard 96-byte DSP-ADPCM header
//!       follows; then the data.
//! ```
//!
//! There is no MIDI-like sequence data and no instrument/note layer: a *call* is a small
//! program of sample words, played back-to-back on one voice. Each word's low 12 bits are a
//! sample index within the bank; the flags are
//!
//! * `0x8000` last word of the call; the three words after it are the parameters
//!   `volume` (0..127), `duck` and `priority`,
//! * `0x2000` after this sample, jump back to the nearest word at or before it with
//! * `0x4000` (loop start).
//!
//! So `S_SELECTMUS` is `4027 0028 0027 0027 0028 a028`: "sel000" (2.9 s) and "sel001"
//! (17.8 s), mono 18 kHz DSP-ADPCM, in the order A B A A B B, looping forever. The shop
//! tunes are `0000 e001` (intro, then a looped body) or `e000` (one looped sample). The map
//! stingers (`S_MAP_*`) and everything else are single samples. A duration of -1 in
//! `AUDATPS2.ROM` marks the looping calls.
//!
//! # Playback semantics (`dcsVoiceStart`, `dcsVoiceSetupAdpcm`, `dcsServiceQueue`)
//!
//! * The voice plays one sample at a time, at the sample's own rate quantised through
//!   `(rate << 12) / 48000` and turned into an AX 16.16 resampling ratio against the 32 kHz
//!   output, with AX linear interpolation. There is no pitch or note mapping.
//! * After every 5 ms AX frame (160 output samples), if the voice has fewer than 0xC0
//!   nibbles left before the end address, the driver reprograms it with the next word's
//!   sample: new start/end/current addresses, ADPCM history and resampler state cleared.
//!   The user callback runs after the DSP has been handed the next frame
//!   (`__AXOutNewFrame`), so the switch lands one frame later, on a frame boundary. At
//!   18 kHz a frame eats 90 input samples, so the old sample's last 0 to 90 samples are
//!   dropped (or, when fewer than one frame was left, the voice runs out and the rest of
//!   that frame is silent). This renderer reproduces that. The samples end in about 25
//!   zero samples, so most joins are clean. Two drop audible music: shop_e1 drops 52
//!   samples and shop_f drops 74, and both click on the real hardware too.
//! * Gain: `volume = p2 * call.volume / 127` with `p2 = sfx_vol * music_vol >> 8`
//!   (`sndFxStartVoice`). This goes through `dcsVoiceSetMaster`'s log approximation to a
//!   master level in 0.1 dB. Pan 127 (centre) gives -3 dB per side through the sndvoice
//!   volume and pan curves. There are no envelopes and no reverb sends. Not modelled: the
//!   ducking that other sounds with a nonzero `duck` apply to every voice while they
//!   play, and the ±8-per-update volume fades of `dcsChannelSetVolPan2`.
//!
//! Because each switch resets all voice state at a frame boundary, a looping call repeats
//! exactly, starting from the frame where its loop-start word was first programmed.

use anyhow::{bail, Result};

use crate::audio::{DspChannel, DspHeader};

/// Low 12 bits of a call word: the sample index within the bank.
pub const WORD_SAMPLE: u16 = 0x0FFF;
/// Loop start marker.
pub const WORD_LOOP_START: u16 = 0x4000;
/// Jump back to the loop start after this sample.
pub const WORD_LOOP_JUMP: u16 = 0x2000;
/// Last sample word of a call (the three parameter words follow).
pub const WORD_LAST: u16 = 0x8000;

/// AX output rate.
pub const OUTPUT_RATE: u32 = 32000;
/// Output samples per AX frame (5 ms), the granularity of the driver's queue service.
pub const FRAME: usize = 160;

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}
fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes(b[o..o + 2].try_into().unwrap())
}

/// One sound ("call") in a bank.
#[derive(Debug, Clone)]
pub struct Call {
    /// Sample words, the last one flagged with [`WORD_LAST`].
    pub words: Vec<u16>,
    /// 0..=127, scaled with the requested volume.
    pub volume: u16,
    /// Amount this sound ducks the other voices while it plays.
    pub duck: u16,
    /// Voice-stealing priority.
    pub priority: u16,
}

impl Call {
    /// Word index the call jumps back to when it reaches a [`WORD_LOOP_JUMP`] word.
    pub fn loop_target(&self) -> Option<usize> {
        let j = self.words.iter().position(|&w| w & WORD_LOOP_JUMP != 0)?;
        Some(loop_back(&self.words, j))
    }

    /// True if the call plays more than one sample or loops.
    pub fn is_sequence(&self) -> bool {
        self.words.len() > 1 || self.loop_target().is_some()
    }

    /// Bank sample index of each word.
    pub fn sample_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().map(|&w| (w & WORD_SAMPLE) as usize)
    }
}

/// The game walks back from the jump word until it finds a loop-start word. With none in
/// the call it would run into the previous call's words; this stops at the first word.
fn loop_back(words: &[u16], mut k: usize) -> usize {
    while k > 0 && words[k] & WORD_LOOP_START == 0 {
        k -= 1;
    }
    k
}

/// One VAG sample in a bank.
#[derive(Debug, Clone)]
pub struct VagSample {
    pub name: String,
    /// Sample rate from the VAG header.
    pub rate: u32,
    pub header: DspHeader,
    pub data_offset: usize,
    /// Data length in bytes (the VAG header's length field, which sets the end address).
    pub data_len: usize,
}

impl VagSample {
    /// Number of samples between the voice's start and end addresses.
    pub fn num_samples(&self) -> usize {
        let nibbles = self.data_len * 2;
        nibbles / 16 * 14 + (nibbles % 16).saturating_sub(2)
    }
}

/// A DCS sound bank (`AUDIO/*.VBK`) parsed the way `dcsBankLoad` reads it.
#[derive(Debug, Clone)]
pub struct DcsBank {
    pub version: u32,
    pub calls: Vec<Call>,
    pub samples: Vec<VagSample>,
}

impl DcsBank {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < 0x14 || &b[0..4] != b"KNBV" {
            bail!("not a KNBV bank");
        }
        let list_size = be32(b, 4) as usize;
        let version = be32(b, 8);
        let ncalls = be32(b, 0x0C) as usize;
        if version & 0xFFFF != 0x106 {
            bail!("unsupported bank type {version:#x}");
        }
        let nsamples = be32(b, 0x10) as usize;
        let list_end = 0x14 + list_size;
        if list_end > b.len() {
            bail!("call list runs past the end of the file");
        }
        let words: Vec<u16> = (0x14..list_end - 1)
            .step_by(2)
            .map(|o| be16(b, o))
            .collect();
        let mut calls = Vec::with_capacity(ncalls);
        let mut i = 0;
        while calls.len() < ncalls {
            let start = i;
            while i < words.len() && words[i] & WORD_LAST == 0 {
                i += 1;
            }
            if i + 3 >= words.len() {
                bail!("call {} runs past the call list", calls.len());
            }
            calls.push(Call {
                words: words[start..=i].to_vec(),
                volume: words[i + 1],
                duck: words[i + 2],
                priority: words[i + 3],
            });
            i += 4;
        }
        if i != words.len() {
            bail!(
                "call list size mismatch ({} of {} words used)",
                i,
                words.len()
            );
        }

        let mut samples = Vec::with_capacity(nsamples);
        let mut o = list_end;
        for k in 0..nsamples {
            if o + 48 > b.len() || &b[o..o + 4] != b"VAGp" {
                bail!("sample {k}: no VAGp header at {o:#x}");
            }
            let ver = be32(b, o + 4);
            let data_len = be32(b, o + 0x0C) as usize;
            let rate = be32(b, o + 0x10);
            let name_end = (o + 0x20..o + 0x30)
                .find(|&p| b[p] == 0)
                .unwrap_or(o + 0x30);
            let name = String::from_utf8_lossy(&b[o + 0x20..name_end]).into_owned();
            o += 48;
            let header = if ver == 0x28 {
                if o + 0x60 > b.len() {
                    bail!("sample {k}: short DSP header");
                }
                let h = DspHeader::parse(&b[o..])?;
                o += 0x60;
                h
            } else {
                bail!("sample {k}: VAG version {ver:#x} is not DSP-ADPCM");
            };
            if o + data_len > b.len() {
                bail!("sample {k}: data runs past the end of the file");
            }
            samples.push(VagSample {
                name,
                rate,
                header,
                data_offset: o,
                data_len,
            });
            o += data_len;
        }
        Ok(DcsBank {
            version,
            calls,
            samples,
        })
    }

    /// Indices of the calls that chain or loop samples.
    pub fn sequenced_calls(&self) -> Vec<usize> {
        (0..self.calls.len())
            .filter(|&i| self.calls[i].is_sequence())
            .collect()
    }

    /// Decode sample `i` from its start address to its end address (mono PCM at
    /// `samples[i].rate`). History starts at zero, as the driver sets it.
    pub fn decode_sample(&self, b: &[u8], i: usize) -> Option<Vec<i16>> {
        let s = self.samples.get(i)?;
        let data = b.get(s.data_offset..s.data_offset + s.data_len)?;
        let mut ch = DspChannel::default();
        let mut out = Vec::with_capacity(s.num_samples() + 14);
        let mut frame = [0u8; 8];
        for chunk in data.chunks(8) {
            frame.fill(0);
            frame[..chunk.len()].copy_from_slice(chunk);
            ch.decode_frame(&s.header.coefs, &frame, &mut out);
        }
        out.truncate(s.num_samples());
        Some(out)
    }
}

/// How the game asks for the sound.
#[derive(Debug, Clone, Copy)]
pub struct PlayParams {
    /// Requested volume `p2` (0..=255) before the call's own volume is applied.
    pub volume: i32,
    /// DCS pan: 0x200 units around the listener, 127 = centre front.
    pub pan: i32,
    /// The `dcsChannelPlay` easter egg that doubles every voice's pitch.
    pub double_pitch: bool,
}

impl PlayParams {
    /// Music as `sndFxPlayHandle(id, music_vol, 1)` starts it, with the options' music
    /// and sfx volumes (0..=255).
    pub fn music(music_vol: i32, sfx_vol: i32) -> Self {
        PlayParams {
            volume: (music_vol.clamp(0, 255) * sfx_vol.clamp(0, 255)) >> 8,
            pan: 127,
            double_pitch: false,
        }
    }
}

impl Default for PlayParams {
    /// The game's default options (music 0x80, sfx 0x80).
    fn default() -> Self {
        PlayParams::music(0x80, 0x80)
    }
}

/// A sample switch during rendering.
#[derive(Debug, Clone)]
pub struct SegmentEvent {
    /// Word index within the call.
    pub word: usize,
    /// Bank sample index.
    pub sample: usize,
    /// Output frame (stereo sample index) where this sample starts.
    pub start: usize,
    /// Input samples played before the voice switched away (or stopped).
    pub played: usize,
    /// Input samples in the sample.
    pub length: usize,
}

/// Rendered call.
#[derive(Debug, Clone)]
pub struct Rendered {
    /// Interleaved stereo i16 at [`OUTPUT_RATE`].
    pub pcm: Vec<i16>,
    pub sample_rate: u32,
    /// For a looping call, the stereo frame the loop restarts at. The loop runs from there
    /// to the end of `pcm` and repeats exactly.
    pub loop_start: Option<usize>,
    pub segments: Vec<SegmentEvent>,
}

impl Rendered {
    /// Number of stereo frames.
    pub fn frames(&self) -> usize {
        self.pcm.len() / 2
    }
}

/// `sndDbTable`: 0.1 dB steps from -96.4 dB to +6 dB; index 904 is 0 dB (0x8000).
#[rustfmt::skip]
static DB_TABLE: [u16; 966] = [
    0x0000, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001,
    0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001,
    0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001,
    0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001,
    0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001,
    0x0001, 0x0001, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002,
    0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002,
    0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002, 0x0002,
    0x0002, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003,
    0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003, 0x0003,
    0x0003, 0x0003, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004,
    0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0004, 0x0005, 0x0005, 0x0005,
    0x0005, 0x0005, 0x0005, 0x0005, 0x0005, 0x0005, 0x0005, 0x0005, 0x0005, 0x0005, 0x0005, 0x0005,
    0x0005, 0x0006, 0x0006, 0x0006, 0x0006, 0x0006, 0x0006, 0x0006, 0x0006, 0x0006, 0x0006, 0x0006,
    0x0006, 0x0006, 0x0007, 0x0007, 0x0007, 0x0007, 0x0007, 0x0007, 0x0007, 0x0007, 0x0007, 0x0007,
    0x0007, 0x0007, 0x0008, 0x0008, 0x0008, 0x0008, 0x0008, 0x0008, 0x0008, 0x0008, 0x0008, 0x0008,
    0x0009, 0x0009, 0x0009, 0x0009, 0x0009, 0x0009, 0x0009, 0x0009, 0x0009, 0x000A, 0x000A, 0x000A,
    0x000A, 0x000A, 0x000A, 0x000A, 0x000A, 0x000A, 0x000B, 0x000B, 0x000B, 0x000B, 0x000B, 0x000B,
    0x000B, 0x000C, 0x000C, 0x000C, 0x000C, 0x000C, 0x000C, 0x000C, 0x000D, 0x000D, 0x000D, 0x000D,
    0x000D, 0x000D, 0x000D, 0x000E, 0x000E, 0x000E, 0x000E, 0x000E, 0x000E, 0x000F, 0x000F, 0x000F,
    0x000F, 0x000F, 0x0010, 0x0010, 0x0010, 0x0010, 0x0010, 0x0011, 0x0011, 0x0011, 0x0011, 0x0011,
    0x0012, 0x0012, 0x0012, 0x0012, 0x0012, 0x0013, 0x0013, 0x0013, 0x0013, 0x0013, 0x0014, 0x0014,
    0x0014, 0x0014, 0x0015, 0x0015, 0x0015, 0x0015, 0x0016, 0x0016, 0x0016, 0x0016, 0x0017, 0x0017,
    0x0017, 0x0018, 0x0018, 0x0018, 0x0018, 0x0019, 0x0019, 0x0019, 0x001A, 0x001A, 0x001A, 0x001A,
    0x001B, 0x001B, 0x001B, 0x001C, 0x001C, 0x001C, 0x001D, 0x001D, 0x001D, 0x001E, 0x001E, 0x001E,
    0x001F, 0x001F, 0x0020, 0x0020, 0x0020, 0x0021, 0x0021, 0x0021, 0x0022, 0x0022, 0x0023, 0x0023,
    0x0023, 0x0024, 0x0024, 0x0025, 0x0025, 0x0026, 0x0026, 0x0026, 0x0027, 0x0027, 0x0028, 0x0028,
    0x0029, 0x0029, 0x002A, 0x002A, 0x002B, 0x002B, 0x002C, 0x002C, 0x002D, 0x002D, 0x002E, 0x002E,
    0x002F, 0x002F, 0x0030, 0x0031, 0x0031, 0x0032, 0x0032, 0x0033, 0x0033, 0x0034, 0x0035, 0x0035,
    0x0036, 0x0037, 0x0037, 0x0038, 0x0038, 0x0039, 0x003A, 0x003A, 0x003B, 0x003C, 0x003D, 0x003D,
    0x003E, 0x003F, 0x003F, 0x0040, 0x0041, 0x0042, 0x0042, 0x0043, 0x0044, 0x0045, 0x0046, 0x0046,
    0x0047, 0x0048, 0x0049, 0x004A, 0x004B, 0x004B, 0x004C, 0x004D, 0x004E, 0x004F, 0x0050, 0x0051,
    0x0052, 0x0053, 0x0054, 0x0055, 0x0056, 0x0057, 0x0058, 0x0059, 0x005A, 0x005B, 0x005C, 0x005D,
    0x005E, 0x005F, 0x0060, 0x0061, 0x0062, 0x0064, 0x0065, 0x0066, 0x0067, 0x0068, 0x006A, 0x006B,
    0x006C, 0x006D, 0x006F, 0x0070, 0x0071, 0x0072, 0x0074, 0x0075, 0x0076, 0x0078, 0x0079, 0x007B,
    0x007C, 0x007E, 0x007F, 0x0080, 0x0082, 0x0083, 0x0085, 0x0087, 0x0088, 0x008A, 0x008B, 0x008D,
    0x008F, 0x0090, 0x0092, 0x0094, 0x0095, 0x0097, 0x0099, 0x009B, 0x009C, 0x009E, 0x00A0, 0x00A2,
    0x00A4, 0x00A6, 0x00A8, 0x00AA, 0x00AB, 0x00AD, 0x00AF, 0x00B2, 0x00B4, 0x00B6, 0x00B8, 0x00BA,
    0x00BC, 0x00BE, 0x00C0, 0x00C3, 0x00C5, 0x00C7, 0x00CA, 0x00CC, 0x00CE, 0x00D1, 0x00D3, 0x00D6,
    0x00D8, 0x00DB, 0x00DD, 0x00E0, 0x00E2, 0x00E5, 0x00E7, 0x00EA, 0x00ED, 0x00F0, 0x00F2, 0x00F5,
    0x00F8, 0x00FB, 0x00FE, 0x0101, 0x0104, 0x0107, 0x010A, 0x010D, 0x0110, 0x0113, 0x0116, 0x011A,
    0x011D, 0x0120, 0x0124, 0x0127, 0x012A, 0x012E, 0x0131, 0x0135, 0x0138, 0x013C, 0x0140, 0x0143,
    0x0147, 0x014B, 0x014F, 0x0153, 0x0157, 0x015B, 0x015F, 0x0163, 0x0167, 0x016B, 0x016F, 0x0173,
    0x0178, 0x017C, 0x0180, 0x0185, 0x0189, 0x018E, 0x0193, 0x0197, 0x019C, 0x01A1, 0x01A6, 0x01AB,
    0x01AF, 0x01B4, 0x01BA, 0x01BF, 0x01C4, 0x01C9, 0x01CE, 0x01D4, 0x01D9, 0x01DF, 0x01E4, 0x01EA,
    0x01EF, 0x01F5, 0x01FB, 0x0201, 0x0207, 0x020D, 0x0213, 0x0219, 0x021F, 0x0226, 0x022C, 0x0232,
    0x0239, 0x0240, 0x0246, 0x024D, 0x0254, 0x025B, 0x0262, 0x0269, 0x0270, 0x0277, 0x027E, 0x0286,
    0x028D, 0x0295, 0x029D, 0x02A4, 0x02AC, 0x02B4, 0x02BC, 0x02C4, 0x02CC, 0x02D5, 0x02DD, 0x02E6,
    0x02EE, 0x02F7, 0x0300, 0x0309, 0x0312, 0x031B, 0x0324, 0x032D, 0x0337, 0x0340, 0x034A, 0x0354,
    0x035D, 0x0367, 0x0371, 0x037C, 0x0386, 0x0390, 0x039B, 0x03A6, 0x03B1, 0x03BB, 0x03C7, 0x03D2,
    0x03DD, 0x03E9, 0x03F4, 0x0400, 0x040C, 0x0418, 0x0424, 0x0430, 0x043D, 0x0449, 0x0456, 0x0463,
    0x0470, 0x047D, 0x048A, 0x0498, 0x04A5, 0x04B3, 0x04C1, 0x04CF, 0x04DD, 0x04EC, 0x04FA, 0x0509,
    0x0518, 0x0527, 0x0536, 0x0546, 0x0555, 0x0565, 0x0575, 0x0586, 0x0596, 0x05A6, 0x05B7, 0x05C8,
    0x05D9, 0x05EB, 0x05FC, 0x060E, 0x0620, 0x0632, 0x0644, 0x0657, 0x066A, 0x067D, 0x0690, 0x06A4,
    0x06B7, 0x06CB, 0x06DF, 0x06F4, 0x0708, 0x071D, 0x0732, 0x0748, 0x075D, 0x0773, 0x0789, 0x079F,
    0x07B6, 0x07CD, 0x07E4, 0x07FB, 0x0813, 0x082B, 0x0843, 0x085C, 0x0874, 0x088E, 0x08A7, 0x08C1,
    0x08DA, 0x08F5, 0x090F, 0x092A, 0x0945, 0x0961, 0x097D, 0x0999, 0x09B5, 0x09D2, 0x09EF, 0x0A0D,
    0x0A2A, 0x0A48, 0x0A67, 0x0A86, 0x0AA5, 0x0AC5, 0x0AE5, 0x0B05, 0x0B25, 0x0B47, 0x0B68, 0x0B8A,
    0x0BAC, 0x0BCF, 0x0BF2, 0x0C15, 0x0C39, 0x0C5D, 0x0C82, 0x0CA7, 0x0CCC, 0x0CF2, 0x0D19, 0x0D3F,
    0x0D67, 0x0D8E, 0x0DB7, 0x0DDF, 0x0E08, 0x0E32, 0x0E5C, 0x0E87, 0x0EB2, 0x0EDD, 0x0F09, 0x0F36,
    0x0F63, 0x0F91, 0x0FBF, 0x0FEE, 0x101D, 0x104D, 0x107D, 0x10AE, 0x10DF, 0x1111, 0x1144, 0x1177,
    0x11AB, 0x11DF, 0x1214, 0x124A, 0x1280, 0x12B7, 0x12EE, 0x1326, 0x135F, 0x1399, 0x13D3, 0x140D,
    0x1449, 0x1485, 0x14C2, 0x14FF, 0x153E, 0x157D, 0x15BC, 0x15FD, 0x163E, 0x1680, 0x16C3, 0x1706,
    0x174A, 0x178F, 0x17D5, 0x181C, 0x1863, 0x18AC, 0x18F5, 0x193F, 0x198A, 0x19D5, 0x1A22, 0x1A6F,
    0x1ABE, 0x1B0D, 0x1B5D, 0x1BAE, 0x1C00, 0x1C53, 0x1CA7, 0x1CFC, 0x1D52, 0x1DA9, 0x1E01, 0x1E5A,
    0x1EB4, 0x1F0F, 0x1F6B, 0x1FC8, 0x2026, 0x2086, 0x20E6, 0x2148, 0x21AA, 0x220E, 0x2273, 0x22D9,
    0x2341, 0x23A9, 0x2413, 0x247E, 0x24EA, 0x2557, 0x25C6, 0x2636, 0x26A7, 0x271A, 0x278E, 0x2803,
    0x287A, 0x28F2, 0x296B, 0x29E6, 0x2A62, 0x2AE0, 0x2B5F, 0x2BDF, 0x2C61, 0x2CE5, 0x2D6A, 0x2DF1,
    0x2E79, 0x2F03, 0x2F8E, 0x301B, 0x30AA, 0x313A, 0x31CC, 0x325F, 0x32F5, 0x338C, 0x3425, 0x34BF,
    0x355B, 0x35FA, 0x369A, 0x373C, 0x37DF, 0x3885, 0x392C, 0x39D6, 0x3A81, 0x3B2F, 0x3BDE, 0x3C90,
    0x3D43, 0x3DF9, 0x3EB1, 0x3F6A, 0x4026, 0x40E5, 0x41A5, 0x4268, 0x432C, 0x43F4, 0x44BD, 0x4589,
    0x4657, 0x4727, 0x47FA, 0x48D0, 0x49A8, 0x4A82, 0x4B5F, 0x4C3E, 0x4D20, 0x4E05, 0x4EEC, 0x4FD6,
    0x50C3, 0x51B2, 0x52A4, 0x5399, 0x5491, 0x558C, 0x5689, 0x578A, 0x588D, 0x5994, 0x5A9D, 0x5BAA,
    0x5CBA, 0x5DCD, 0x5EE3, 0x5FFC, 0x6119, 0x6238, 0x635C, 0x6482, 0x65AC, 0x66D9, 0x680A, 0x693F,
    0x6A77, 0x6BB2, 0x6CF2, 0x6E35, 0x6F7B, 0x70C6, 0x7214, 0x7366, 0x74BC, 0x7616, 0x7774, 0x78D6,
    0x7A3D, 0x7BA7, 0x7D16, 0x7E88, 0x7FFF, 0x817B, 0x82FB, 0x847F, 0x8608, 0x8795, 0x8927, 0x8ABE,
    0x8C59, 0x8DF9, 0x8F9E, 0x9148, 0x92F6, 0x94AA, 0x9663, 0x9820, 0x99E3, 0x9BAB, 0x9D79, 0x9F4C,
    0xA124, 0xA302, 0xA4E5, 0xA6CE, 0xA8BC, 0xAAB0, 0xACAA, 0xAEAA, 0xB0B0, 0xB2BC, 0xB4CE, 0xB6E5,
    0xB904, 0xBB28, 0xBD53, 0xBF84, 0xC1BC, 0xC3FA, 0xC63F, 0xC88B, 0xCADD, 0xCD37, 0xCF97, 0xD1FE,
    0xD46D, 0xD6E3, 0xD960, 0xDBE4, 0xDE70, 0xE103, 0xE39E, 0xE641, 0xE8EB, 0xEB9E, 0xEE58, 0xF11B,
    0xF3E6, 0xF6B9, 0xF994, 0xFC78, 0xFF64, 0x0000,
];

/// `sndVolCurve`; `sndPanCurve[i]` equals `sndVolCurve[127 - i]`.
#[rustfmt::skip]
static VOL_CURVE: [i32; 128] = [
    0, 0, -1, -1, -1, -2, -2, -2, -3, -3, -4, -4, -4, -5, -5, -5,
    -6, -6, -7, -7, -7, -8, -8, -9, -9, -10, -10, -10, -11, -11, -12, -12,
    -13, -13, -14, -14, -14, -15, -15, -16, -16, -17, -17, -18, -18, -19, -20, -20,
    -21, -21, -22, -22, -23, -23, -24, -25, -25, -26, -26, -27, -28, -28, -29, -30,
    -30, -31, -32, -33, -33, -34, -35, -36, -36, -37, -38, -39, -40, -40, -41, -42,
    -43, -44, -45, -46, -47, -48, -49, -50, -51, -52, -54, -55, -56, -57, -59, -60,
    -61, -63, -64, -66, -67, -69, -71, -72, -74, -76, -78, -80, -83, -85, -87, -90,
    -93, -96, -99, -102, -106, -110, -115, -120, -126, -133, -140, -150, -163, -180, -210, -904,
];

fn pan_curve(i: usize) -> i32 {
    VOL_CURVE[127 - i]
}

/// `sndDbToMix`: 0.1 dB to an AX 1.15 mixer gain.
pub fn db_to_mix(db: i32) -> u16 {
    if db <= -0x388 {
        0
    } else if db >= 0x3C {
        0xFF64
    } else {
        DB_TABLE[(db + 0x388) as usize]
    }
}

/// `dcsVoiceSetMaster`: linear 0..=0x3FFF to 0.1 dB (-3 dB per halving, -1000 for 0).
pub fn master_db(level: i32) -> i32 {
    if level >= 0x3FFF {
        return 0;
    }
    if level <= 0 {
        return -1000;
    }
    let mut bit = 0x2000;
    let mut master = -30;
    while level & bit == 0 {
        bit >>= 1;
        master -= 30;
    }
    master + ((bit >> 1) + (level - bit) * 30) / bit
}

/// Left/right AX mixer gains (1.15) for a DCS voice at `volume` (0..=255) and `pan`.
pub fn voice_gains(volume: i32, pan: i32) -> (u16, u16) {
    let level = (volume.max(0) * 0x3FFF / 0xFF).min(0x3FFF);
    let master = master_db(level);
    let pan = pan as i16 as i32;
    let vol_idx = ((0x100 - ((pan + 0x100) & 0x1FF)).abs() >> 1).clamp(0, 127) as usize;
    let pan_idx = ((0x100 - ((pan + 0x180) & 0x1FF)).abs() >> 1).clamp(0, 127) as usize;
    let (vol_l, vol_r, pan_l) = (VOL_CURVE[vol_idx], pan_curve(vol_idx), pan_curve(pan_idx));
    (
        db_to_mix(master + vol_l + pan_l),
        db_to_mix(master + vol_r + pan_l),
    )
}

/// The AX 16.16 resampling ratio `dcsVoiceStartAx` programs for a sample rate.
pub fn src_ratio(rate: u32, double_pitch: bool) -> u32 {
    let q = (rate << 12) / 48000;
    let q = q << u32::from(double_pitch);
    let mut ratio = (((q as u64 * 48000) >> 12) as u32) as f32 / 32000.0;
    let hi = ratio as i32;
    ratio -= hi as f32;
    let lo = (65536.0f64 * ratio as f64) as i32;
    ((hi as u32 & 0xFFFF) << 16) | (lo as u32 & 0xFFFF)
}

/// Nibble address of sample `j` relative to the sample's start, frame headers included.
fn nibble_addr(j: usize) -> usize {
    j / 14 * 16 + j % 14
}

/// One AX voice playing DSP-ADPCM with linear sample-rate conversion.
struct Voice {
    pcm: Vec<i16>,
    next: usize,
    running: bool,
    ratio: u32,
    frac: u32,
    last: [i16; 4],
}

impl Voice {
    fn new(pcm: Vec<i16>, ratio: u32) -> Self {
        let running = !pcm.is_empty();
        Voice {
            pcm,
            next: 0,
            running,
            ratio,
            frac: 0,
            last: [0; 4],
        }
    }

    fn fetch(&mut self) -> i16 {
        if !self.running {
            return 0;
        }
        let s = self.pcm[self.next];
        if self.next + 1 >= self.pcm.len() {
            // Reached the end address with looping off: the voice stops.
            self.running = false;
        } else {
            self.next += 1;
        }
        s
    }

    /// Nibbles between the current and end addresses, as `dcsServiceQueue` measures.
    fn remaining_nibbles(&self) -> usize {
        if !self.running {
            return 0;
        }
        nibble_addr(self.pcm.len() - 1) - nibble_addr(self.next)
    }

    /// Input samples consumed so far.
    fn played(&self) -> usize {
        if self.running {
            self.next
        } else {
            self.pcm.len()
        }
    }

    /// Render one AX frame of mono output.
    fn frame(&mut self, out: &mut [i32; FRAME]) {
        let needed = ((self.frac as u64 + self.ratio as u64 * FRAME as u64) >> 16) as usize;
        let mut temp = Vec::with_capacity(needed + 4);
        temp.extend_from_slice(&self.last);
        for _ in 0..needed {
            let s = self.fetch();
            temp.push(s);
        }
        // Linear SRC: at integer position k (input samples consumed so far this frame) the
        // output lies between the two newest consumed samples, temp[k + 2] and temp[k + 3].
        let mut pos = self.frac;
        for o in out.iter_mut() {
            let k = (pos >> 16) as usize;
            let f = (pos & 0xFFFF) as i64;
            let (s0, s1) = (temp[k + 2] as i64, temp[k + 3] as i64);
            *o = (s0 as i16).wrapping_add((((s1 - s0) * f) >> 16) as i16) as i32;
            pos = pos.wrapping_add(self.ratio);
        }
        self.frac = pos & 0xFFFF;
        self.last.copy_from_slice(&temp[needed..needed + 4]);
    }
}

/// Plays one call frame by frame, as the DCS driver and the AX voice do. Looping calls
/// play forever; use [`render_call`] for one pass and the loop point.
pub struct CallPlayer<'a> {
    bank: &'a DcsBank,
    bytes: &'a [u8],
    call: &'a Call,
    double_pitch: bool,
    gains: (i32, i32),
    ve: i32,
    decoded: Vec<Option<Vec<i16>>>,
    voice: Voice,
    /// Word the driver programs next (its sample slot); None after the last word.
    pending: Option<usize>,
    /// Reprogram decided in the callback after frame N; it reaches the DSP at the sync
    /// before frame N + 2, because frame N + 1 was already handed over: (word, frame).
    scheduled: Option<(usize, usize)>,
    frame: usize,
    first_frame: Vec<Option<usize>>,
    /// Every sample switch so far.
    pub segments: Vec<SegmentEvent>,
    /// Set when the call first jumps back to a word it already played: the stereo frame
    /// where that word first started.
    pub loop_start: Option<usize>,
    /// Stereo frame where that first jump back took effect (the length of one pass).
    pub loop_end: Option<usize>,
    finished: bool,
}

impl<'a> CallPlayer<'a> {
    pub fn new(
        bank: &'a DcsBank,
        bytes: &'a [u8],
        call: usize,
        params: &PlayParams,
    ) -> Result<Self> {
        let Some(c) = bank.calls.get(call) else {
            bail!("no call {call} in bank");
        };
        if let Some(&w) = c
            .words
            .iter()
            .find(|&&w| (w & WORD_SAMPLE) as usize >= bank.samples.len())
        {
            bail!("call {call}: sample {} not in bank", w & WORD_SAMPLE);
        }
        let volume = params.volume * c.volume as i32 / 0x7F;
        let (l, r) = voice_gains(volume, params.pan);
        let mut p = CallPlayer {
            bank,
            bytes,
            call: c,
            double_pitch: params.double_pitch,
            gains: (l as i32, r as i32),
            // The voice envelope stays at sndDbToMix(0) for DCS voices.
            ve: db_to_mix(0) as i32,
            decoded: vec![None; bank.samples.len()],
            voice: Voice::new(Vec::new(), 0),
            pending: None,
            scheduled: None,
            frame: 0,
            first_frame: vec![None; c.words.len()],
            segments: Vec::new(),
            loop_start: None,
            loop_end: None,
            finished: false,
        };
        p.program(0);
        Ok(p)
    }

    /// `dcsVoiceSetupAdpcm`: start the word's sample and pick the word after it.
    fn program(&mut self, word: usize) {
        if let Some(prev) = self.segments.last_mut() {
            prev.played = self.voice.played();
        }
        match self.first_frame[word] {
            Some(f) if self.loop_start.is_none() => {
                self.loop_start = Some(f * FRAME);
                self.loop_end = Some(self.frame * FRAME);
            }
            Some(_) => {}
            None => self.first_frame[word] = Some(self.frame),
        }
        let w = self.call.words[word];
        let si = (w & WORD_SAMPLE) as usize;
        let pcm = self.decoded[si]
            .get_or_insert_with(|| self.bank.decode_sample(self.bytes, si).unwrap_or_default())
            .clone();
        self.pending = if w & WORD_LOOP_JUMP != 0 {
            Some(loop_back(&self.call.words, word))
        } else if w & WORD_LAST != 0 {
            None
        } else {
            Some(word + 1)
        };
        let ratio = src_ratio(self.bank.samples[si].rate, self.double_pitch);
        self.segments.push(SegmentEvent {
            word,
            sample: si,
            start: self.frame * FRAME,
            played: 0,
            length: pcm.len(),
        });
        self.voice = Voice::new(pcm, ratio);
    }

    /// True once a one-shot call has played out.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Stereo frames rendered so far.
    pub fn position(&self) -> usize {
        self.frame * FRAME
    }

    /// Render one 5 ms AX frame ([`FRAME`] stereo frames, interleaved) onto `out`.
    /// Returns false, adding nothing, once the call has finished.
    pub fn render_frame(&mut self, out: &mut Vec<i16>) -> bool {
        if self.finished {
            return false;
        }
        if let Some((word, at)) = self.scheduled {
            if at == self.frame {
                self.scheduled = None;
                self.program(word);
            }
        }
        if self.scheduled.is_none() && self.pending.is_none() && !self.voice.running {
            if let Some(s) = self.segments.last_mut() {
                s.played = self.voice.played();
            }
            self.finished = true;
            return false;
        }
        let mut buf = [0i32; FRAME];
        self.voice.frame(&mut buf);
        for &s in &buf {
            let s = (s * self.ve) >> 15;
            out.push(((s * self.gains.0) >> 15).clamp(-32768, 32767) as i16);
            out.push(((s * self.gains.1) >> 15).clamp(-32768, 32767) as i16);
        }
        // The AX frame callback (dcsServiceQueue). While a reprogram is waiting, the
        // CPU-side addresses are the new sample's start, so it cannot fire twice.
        if self.scheduled.is_none() {
            if let Some(word) = self.pending {
                if self.voice.remaining_nibbles() < 0xC0 {
                    self.scheduled = Some((word, self.frame + 2));
                }
            }
        }
        self.frame += 1;
        true
    }
}

/// Longest render for a call that neither ends nor loops back (it cannot happen with a
/// well-formed call; this only bounds the loop).
const MAX_FRAMES: usize = OUTPUT_RATE as usize / FRAME * 60 * 30;

/// Render call `call` of `bank` (file bytes `b`) as the game's DCS driver plays it.
///
/// One-shot calls render until the voice stops. Looping calls render one pass, up to the
/// point where the loop restarts; `loop_start` says where the repeat begins.
pub fn render_call(bank: &DcsBank, b: &[u8], call: usize, params: &PlayParams) -> Result<Rendered> {
    let mut player = CallPlayer::new(bank, b, call, params)?;
    let mut pcm = Vec::new();
    for _ in 0..MAX_FRAMES {
        let before = pcm.len();
        if !player.render_frame(&mut pcm) {
            break;
        }
        if let Some(end) = player.loop_end {
            // The loop came round at the start of this frame: one pass is complete.
            debug_assert_eq!(end * 2, before);
            pcm.truncate(end * 2);
            break;
        }
    }
    let mut segments = player.segments.clone();
    if player.loop_end.is_some() {
        segments.pop();
    }
    Ok(Rendered {
        pcm,
        sample_rate: OUTPUT_RATE,
        loop_start: player.loop_start,
        segments,
    })
}
