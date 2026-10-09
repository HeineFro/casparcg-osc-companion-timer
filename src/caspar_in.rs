//! CasparCG's OSC output translated to `ClipUpdate`.
//!
//! We only read `/channel/<c>/stage/layer/<l>/foreground/file/{name,time}`
//! and only for layers that have a binding (`watched`). Everything else is
//! dropped before its arguments are even looked at.

use rosc::{OscMessage, OscPacket, OscType};

use crate::layer_state::{ClipUpdate, LayerKey};

pub type LayerUpdate = (LayerKey, ClipUpdate);

/// Decodes one UDP datagram. Invalid datagrams yield an empty list.
pub fn decode_updates(datagram: &[u8], watched: &dyn Fn(LayerKey) -> bool) -> Vec<LayerUpdate> {
    let mut updates = Vec::new();
    if let Ok((_, packet)) = rosc::decoder::decode_udp(datagram) {
        collect_updates(&packet, watched, &mut updates);
    }
    updates
}

fn collect_updates(
    packet: &OscPacket,
    watched: &dyn Fn(LayerKey) -> bool,
    out: &mut Vec<LayerUpdate>,
) {
    match packet {
        OscPacket::Message(message) => out.extend(parse_message(message, watched)),
        OscPacket::Bundle(bundle) => {
            for inner in &bundle.content {
                collect_updates(inner, watched, out);
            }
        }
    }
}

fn parse_message(message: &OscMessage, watched: &dyn Fn(LayerKey) -> bool) -> Option<LayerUpdate> {
    let (key, leaf) = parse_address(&message.addr)?;
    if !watched(key) {
        return None;
    }
    let update = match leaf {
        Leaf::Name => ClipUpdate::Name(name_from(&message.args)),
        Leaf::Time => time_from(&message.args)?,
    };
    Some((key, update))
}

enum Leaf {
    Name,
    Time,
}

fn parse_address(address: &str) -> Option<(LayerKey, Leaf)> {
    let mut parts = address.strip_prefix('/')?.split('/');
    expect(&mut parts, "channel")?;
    let channel = number(&mut parts)?;
    expect(&mut parts, "stage")?;
    expect(&mut parts, "layer")?;
    let layer = number(&mut parts)?;
    expect(&mut parts, "foreground")?;
    expect(&mut parts, "file")?;
    let leaf = match parts.next()? {
        "name" => Leaf::Name,
        "time" => Leaf::Time,
        _ => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((LayerKey { channel, layer }, leaf))
}

fn expect<'a>(parts: &mut impl Iterator<Item = &'a str>, word: &str) -> Option<()> {
    (parts.next()? == word).then_some(())
}

fn number<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Option<u32> {
    parts.next()?.parse().ok()
}

fn name_from(args: &[OscType]) -> Option<String> {
    match args.first() {
        Some(OscType::String(name)) if !name.is_empty() => Some(name.clone()),
        _ => None,
    }
}

fn time_from(args: &[OscType]) -> Option<ClipUpdate> {
    let elapsed = seconds(args.first()?)?;
    let total = seconds(args.get(1)?)?;
    Some(ClipUpdate::Time { elapsed, total })
}

fn seconds(arg: &OscType) -> Option<f64> {
    match arg {
        OscType::Float(value) => Some(f64::from(*value)),
        OscType::Double(value) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{encoder, OscBundle, OscTime};

    fn all(_: LayerKey) -> bool {
        true
    }

    fn message(addr: &str, args: Vec<OscType>) -> OscPacket {
        OscPacket::Message(OscMessage { addr: addr.into(), args })
    }

    fn bytes(packet: &OscPacket) -> Vec<u8> {
        encoder::encode(packet).unwrap()
    }

    #[test]
    fn parses_name_and_time() {
        let name = message(
            "/channel/1/stage/layer/10/foreground/file/name",
            vec![OscType::String("AMB".into())],
        );
        let time = message(
            "/channel/1/stage/layer/10/foreground/file/time",
            vec![OscType::Float(3.5), OscType::Float(10.0)],
        );
        let key = LayerKey { channel: 1, layer: 10 };

        assert_eq!(
            decode_updates(&bytes(&name), &all),
            vec![(key, ClipUpdate::Name(Some("AMB".into())))]
        );
        assert_eq!(
            decode_updates(&bytes(&time), &all),
            vec![(key, ClipUpdate::Time { elapsed: 3.5, total: 10.0 })]
        );
    }

    #[test]
    fn empty_name_means_empty_layer() {
        let name = message(
            "/channel/1/stage/layer/10/foreground/file/name",
            vec![OscType::String(String::new())],
        );
        let updates = decode_updates(&bytes(&name), &all);
        assert_eq!(updates[0].1, ClipUpdate::Name(None));
    }

    #[test]
    fn unwatched_layers_are_dropped() {
        let time = message(
            "/channel/1/stage/layer/11/foreground/file/time",
            vec![OscType::Float(1.0), OscType::Float(2.0)],
        );
        let only_layer_10 = |key: LayerKey| key.layer == 10;
        assert!(decode_updates(&bytes(&time), &only_layer_10).is_empty());
    }

    #[test]
    fn other_addresses_are_ignored() {
        for addr in [
            "/channel/1/stage/layer/10/foreground/paused",
            "/channel/1/stage/layer/10/background/file/name",
            "/channel/1/stage/layer/10/foreground/file/name/extra",
            "/channel/x/stage/layer/10/foreground/file/name",
            "/channel/1/mixer/audio/volume",
        ] {
            let packet = message(addr, vec![OscType::String("x".into())]);
            assert!(decode_updates(&bytes(&packet), &all).is_empty(), "{addr}");
        }
    }

    #[test]
    fn time_needs_two_numbers() {
        let packet = message(
            "/channel/1/stage/layer/10/foreground/file/time",
            vec![OscType::Float(1.0)],
        );
        assert!(decode_updates(&bytes(&packet), &all).is_empty());
    }

    #[test]
    fn bundles_are_unpacked() {
        let bundle = OscPacket::Bundle(OscBundle {
            timetag: OscTime { seconds: 0, fractional: 1 },
            content: vec![
                message(
                    "/channel/1/stage/layer/10/foreground/file/name",
                    vec![OscType::String("AMB".into())],
                ),
                message(
                    "/channel/1/stage/layer/10/foreground/file/time",
                    vec![OscType::Float(1.0), OscType::Float(2.0)],
                ),
            ],
        });
        assert_eq!(decode_updates(&bytes(&bundle), &all).len(), 2);
    }

    #[test]
    fn garbage_is_ignored() {
        assert!(decode_updates(b"not osc", &all).is_empty());
    }
}
