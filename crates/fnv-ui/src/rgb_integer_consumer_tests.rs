//! Authored RGB consumers: only nonnegative, exactly representable integer colours.

use crate::{MemSource, Ui};

fn source() -> MemSource {
    MemSource::default().with(
        "menus/rgb_fixture.xml",
        r#"<menu name="RgbFixture">
          <rect name="Option">
            <_input> 1125339 </_input>
            <_channelR><copy src="me()" trait="_input"/><shr>16</shr><land>255</land></_channelR>
            <_channelG><copy src="me()" trait="_input"/><shr>8</shr><land>255</land></_channelG>
            <_channelB><copy src="me()" trait="_input"/><land>255</land></_channelB>
            <image name="Swatch">
              <red><copy src="parent()" trait="_channelR"/></red>
              <green><copy src="parent()" trait="_channelG"/></green>
              <blue><copy src="parent()" trait="_channelB"/></blue>
            </image>
          </rect>
          <rect name="Picker">
            <_r>17</_r><_g>43</_g><_b>219</_b>
            <_output>
              <copy src="me()" trait="_r"/><shl>16</shl>
              <add><copy src="me()" trait="_g"/><shl>8</shl></add>
              <add><copy src="me()" trait="_b"/><shl>0</shl></add>
            </_output>
          </rect>
          <rect name="Neighbor"><width><copy>31</copy><add>12</add></width></rect>
        </menu>"#,
    )
}

fn loaded() -> Ui {
    let mut ui = Ui::new();
    ui.load_menu(&source(), "menus/rgb_fixture.xml").unwrap();
    ui.update();
    ui
}

fn channels(ui: &Ui) -> [f64; 3] {
    ["red", "green", "blue"].map(|name| {
        ui.get_float(&format!("RgbFixture/Option/Swatch/{name}"))
            .unwrap()
    })
}

fn packed([r, g, b]: [u8; 3]) -> f64 {
    // The independent expected value is ordinary integer place-value arithmetic.
    f64::from(u32::from(r) * 65536 + u32::from(g) * 256 + u32::from(b))
}

#[test]
fn packed_option_value_reaches_distinct_swatch_channels_including_endpoints() {
    let mut ui = loaded();
    for rgb in [
        [17, 43, 219],
        [0, 0, 0],
        [255, 255, 255],
        [255, 0, 128],
        [0, 255, 1],
    ] {
        assert!(ui.set_float("RgbFixture/Option/_input", packed(rgb)));
        ui.update();
        assert_eq!(
            channels(&ui),
            rgb.map(f64::from),
            "packed option must reach the actual image channels for {rgb:?}"
        );
        assert_eq!(ui.get_float("RgbFixture/Neighbor/width"), Some(43.0));
    }
    assert!(
        ui.warnings.is_empty(),
        "supported RGB expressions must parse: {:?}",
        ui.warnings
    );
}

#[test]
fn picker_channels_reach_the_packed_value_without_logical_and_or_channel_aliasing() {
    let mut ui = loaded();
    for rgb in [
        [17, 43, 219],
        [0, 0, 0],
        [255, 255, 255],
        [255, 0, 128],
        [0, 255, 1],
    ] {
        for (channel, value) in ["_r", "_g", "_b"].into_iter().zip(rgb) {
            assert!(ui.set_float(&format!("RgbFixture/Picker/{channel}"), f64::from(value)));
        }
        ui.update();
        assert_eq!(
            ui.get_float("RgbFixture/Picker/_output"),
            Some(packed(rgb)),
            "picker must pack independently changed channels for {rgb:?}"
        );
    }
    assert!(ui.warnings.is_empty(), "{:?}", ui.warnings);
}

#[test]
fn selected_colour_roundtrip_live_changes_and_reentry_keep_the_color_consumers_consistent() {
    let mut ui = loaded();
    for rgb in [[29, 173, 82], [204, 5, 153], [0, 255, 255]] {
        for (channel, value) in ["_r", "_g", "_b"].into_iter().zip(rgb) {
            ui.set_float(&format!("RgbFixture/Picker/{channel}"), f64::from(value));
        }
        ui.update();
        let output = ui.get_float("RgbFixture/Picker/_output").unwrap();
        assert_eq!(
            output,
            packed(rgb),
            "edited value must reach packed output before applying it"
        );
        ui.set_float("RgbFixture/Option/_input", output);
        ui.update();
        assert_eq!(channels(&ui), rgb.map(f64::from));
        assert_eq!(ui.update(), 0, "an unchanged colour must settle");
        // A newly loaded tree receives the chosen value through the ordinary setter.
        let mut reopened = loaded();
        reopened.set_float("RgbFixture/Option/_input", output);
        reopened.update();
        assert_eq!(channels(&reopened), rgb.map(f64::from));
        assert_eq!(reopened.get_float("RgbFixture/Neighbor/width"), Some(43.0));
    }
}
