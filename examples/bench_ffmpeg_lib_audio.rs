#[cfg(feature = "ffmpeg-audio")]
extern crate ffmpeg_next as ffmpeg;

#[cfg(feature = "ffmpeg-audio")]
use anyhow::{Context, Result};
#[cfg(feature = "ffmpeg-audio")]
use ffmpeg::{codec, filter, format, frame, media, Dictionary};
#[cfg(feature = "ffmpeg-audio")]
use rayon::prelude::*;
#[cfg(feature = "ffmpeg-audio")]
use std::path::{Path, PathBuf};
#[cfg(feature = "ffmpeg-audio")]
use std::time::Instant;

#[cfg(feature = "ffmpeg-audio")]
const TARGET_RATE: i32 = 24_000;
#[cfg(feature = "ffmpeg-audio")]
const VORBIS_Q: usize = 5 * 118;

#[cfg(feature = "ffmpeg-audio")]
struct Transcoder {
    stream: usize,
    filter: filter::Graph,
    decoder: codec::decoder::Audio,
    encoder: codec::encoder::Audio,
    in_time_base: ffmpeg::Rational,
    out_time_base: ffmpeg::Rational,
}

#[cfg(feature = "ffmpeg-audio")]
fn build_filter(
    decoder: &codec::decoder::Audio,
    encoder: &codec::encoder::Audio,
) -> Result<filter::Graph, ffmpeg::Error> {
    let mut graph = filter::Graph::new();
    let args = format!(
        "time_base={}:sample_rate={}:sample_fmt={}:channel_layout=0x{:x}",
        decoder.time_base(),
        decoder.rate(),
        decoder.format().name(),
        decoder.channel_layout().bits()
    );

    graph.add(&filter::find("abuffer").unwrap(), "in", &args)?;
    graph.add(&filter::find("abuffersink").unwrap(), "out", "")?;

    {
        let mut out = graph.get("out").unwrap();
        out.set_sample_format(encoder.format());
        out.set_channel_layout(encoder.channel_layout());
        out.set_sample_rate(encoder.rate());
    }

    graph
        .output("in", 0)?
        .input("out", 0)?
        .parse("aresample=24000")?;
    graph.validate()?;

    if let Some(codec) = encoder.codec() {
        if !codec
            .capabilities()
            .contains(ffmpeg::codec::capabilities::Capabilities::VARIABLE_FRAME_SIZE)
        {
            graph
                .get("out")
                .unwrap()
                .sink()
                .set_frame_size(encoder.frame_size());
        }
    }

    Ok(graph)
}

#[cfg(feature = "ffmpeg-audio")]
fn make_transcoder(
    ictx: &mut format::context::Input,
    octx: &mut format::context::Output,
) -> Result<Transcoder, ffmpeg::Error> {
    let input = ictx
        .streams()
        .best(media::Type::Audio)
        .expect("could not find audio stream");
    let context = ffmpeg::codec::context::Context::from_parameters(input.parameters())?;
    let mut decoder = context.decoder().audio()?;
    decoder.set_parameters(input.parameters())?;

    let codec = ffmpeg::encoder::find_by_name("libvorbis")
        .or_else(|| ffmpeg::encoder::find(codec::Id::VORBIS))
        .expect("failed to find Vorbis encoder")
        .audio()?;

    let global = octx
        .format()
        .flags()
        .contains(ffmpeg::format::flag::Flags::GLOBAL_HEADER);
    let mut output = octx.add_stream(codec)?;
    let context = ffmpeg::codec::context::Context::from_parameters(output.parameters())?;
    let mut encoder = context.encoder().audio()?;

    let channel_layout = decoder.channel_layout();
    if global {
        encoder.set_flags(ffmpeg::codec::flag::Flags::GLOBAL_HEADER);
    }
    encoder.set_flags(ffmpeg::codec::flag::Flags::QSCALE);
    encoder.set_quality(VORBIS_Q);
    encoder.set_rate(TARGET_RATE);
    encoder.set_channel_layout(channel_layout);
    encoder.set_format(
        codec
            .formats()
            .expect("unknown supported formats")
            .next()
            .unwrap(),
    );
    encoder.set_time_base((1, TARGET_RATE));
    output.set_time_base((1, TARGET_RATE));

    let encoder = encoder.open_as_with(codec, Dictionary::new())?;
    output.set_parameters(&encoder);

    let filter = build_filter(&decoder, &encoder)?;
    let in_time_base = decoder.time_base();
    let out_time_base = output.time_base();

    Ok(Transcoder {
        stream: input.index(),
        filter,
        decoder,
        encoder,
        in_time_base,
        out_time_base,
    })
}

impl Transcoder {
    fn receive_packets(&mut self, octx: &mut format::context::Output) -> Result<(), ffmpeg::Error> {
        let mut encoded = ffmpeg::Packet::empty();
        while self.encoder.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(0);
            encoded.rescale_ts(self.in_time_base, self.out_time_base);
            encoded.write_interleaved(octx)?;
        }
        Ok(())
    }

    fn receive_filtered(
        &mut self,
        octx: &mut format::context::Output,
    ) -> Result<(), ffmpeg::Error> {
        let mut filtered = frame::Audio::empty();
        while self
            .filter
            .get("out")
            .unwrap()
            .sink()
            .frame(&mut filtered)
            .is_ok()
        {
            self.encoder.send_frame(&filtered)?;
            self.receive_packets(octx)?;
        }
        Ok(())
    }

    fn receive_decoded(&mut self, octx: &mut format::context::Output) -> Result<(), ffmpeg::Error> {
        let mut decoded = frame::Audio::empty();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            let timestamp = decoded.timestamp();
            decoded.set_pts(timestamp);
            self.filter.get("in").unwrap().source().add(&decoded)?;
            self.receive_filtered(octx)?;
        }
        Ok(())
    }
}

#[cfg(feature = "ffmpeg-audio")]
fn transcode_file(input: &Path, output: &Path) -> Result<()> {
    let mut ictx = format::input(input)?;
    let mut octx = format::output(output)?;
    let mut transcoder = make_transcoder(&mut ictx, &mut octx)?;

    octx.write_header()?;

    for (stream, mut packet) in ictx.packets() {
        if stream.index() == transcoder.stream {
            packet.rescale_ts(stream.time_base(), transcoder.in_time_base);
            transcoder.decoder.send_packet(&packet)?;
            transcoder.receive_decoded(&mut octx)?;
        }
    }

    transcoder.decoder.send_eof()?;
    transcoder.receive_decoded(&mut octx)?;

    transcoder.filter.get("in").unwrap().source().flush()?;
    transcoder.receive_filtered(&mut octx)?;

    transcoder.encoder.send_eof()?;
    transcoder.receive_packets(&mut octx)?;

    octx.write_trailer()?;
    Ok(())
}

#[cfg(feature = "ffmpeg-audio")]
fn main() -> Result<()> {
    ffmpeg::init().context("failed to initialize ffmpeg")?;

    let mut args = std::env::args_os().skip(1);
    let input_dir = PathBuf::from(
        args.next()
            .context("usage: bench_ffmpeg_lib_audio <input-dir> <output-dir> [limit]")?,
    );
    let output_dir = PathBuf::from(
        args.next()
            .context("usage: bench_ffmpeg_lib_audio <input-dir> <output-dir> [limit]")?,
    );
    let limit = args
        .next()
        .and_then(|value| value.to_string_lossy().parse::<usize>().ok());

    std::fs::create_dir_all(&output_dir)?;
    let mut inputs: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ogg"))
        })
        .collect();
    inputs.sort();
    if let Some(limit) = limit {
        inputs.truncate(limit);
    }

    let start = Instant::now();
    inputs.par_iter().try_for_each(|input| -> Result<()> {
        let output = output_dir.join(input.file_name().context("input missing file name")?);
        transcode_file(input, &output)
            .with_context(|| format!("failed to transcode {}", input.display()))
    })?;
    let elapsed = start.elapsed();
    let total_bytes: u64 = inputs
        .iter()
        .map(|input| {
            output_dir
                .join(input.file_name().unwrap())
                .metadata()
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        })
        .sum();

    println!(
        "count={} elapsed_seconds={:.3} output_bytes={}",
        inputs.len(),
        elapsed.as_secs_f64(),
        total_bytes
    );

    Ok(())
}

#[cfg(not(feature = "ffmpeg-audio"))]
fn main() {
    eprintln!("Rebuild with --features ffmpeg-audio to run this benchmark.");
}
