use anyhow::Context;
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{debug, trace, warn};

use bytes::BytesMut;
use usb_gadget::function::custom;
use usb_gadget::function::custom::{CtrlSender, Endpoint, EndpointDirection, EndpointReceiver};
use usb_gadget::Id;

const GUD_DISPLAY_MAGIC: u32 = 0x1d50614d;

const GUD_REQ_GET_STATUS: u8 = 0x00;
const GUD_REQ_GET_DESCRIPTOR: u8 = 0x01;
const GUD_REQ_GET_FORMATS: u8 = 0x40;
const GUD_REQ_GET_PROPERTIES: u8 = 0x41;
const GUD_REQ_GET_CONNECTORS: u8 = 0x50;
const GUD_REQ_GET_CONNECTOR_PROPERTIES: u8 = 0x51;
const GUD_REQ_GET_CONNECTOR_STATUS: u8 = 0x54;
const GUD_REQ_GET_CONNECTOR_MODES: u8 = 0x55;
const GUD_REQ_GET_CONNECTOR_EDID: u8 = 0x56;

const GUD_REQ_SET_CONNECTOR_FORCE_DETECT: u8 = 0x53;
const GUD_REQ_SET_BUFFER: u8 = 0x60;
const GUD_REQ_SET_STATE_CHECK: u8 = 0x61;
const GUD_REQ_SET_STATE_COMMIT: u8 = 0x62;
const GUD_REQ_SET_CONTROLLER_ENABLE: u8 = 0x63;
const GUD_REQ_SET_DISPLAY_ENABLE: u8 = 0x64;

// https://github.com/openmoko/openmoko-usb-oui/commit/73bdf541b6f9840b70219626b4088d4e3f164904
pub const OPENMOKO_GUD_ID: Id = Id::new(0x1d50, 0x614d);

#[repr(u8)]
#[derive(Debug, Serialize, Deserialize)]
enum Status {
    Ok = 0x00,
    Busy = 0x01,
    RequestNotSupported = 0x02,
    ProtocolError = 0x03,
    InvalidParameter = 0x04,
    Error = 0x05,
}

#[repr(transparent)]
#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectorStatus(u8);

bitflags! {
    impl ConnectorStatus: u8{
        const DISCONNECTED = 0x00;
        const CONNECTED = 0x01;
        const UNKNOWN = 0x02;

        const CHANGED = 1 << 7;

        // The source may set any bits
        const _ = !0;
    }
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PixelFormat {
    /// 1-bit monochrome
    R1 = 0x01,
    /// 8-bit greyscale
    R8 = 0x08,
    XRGB1111 = 0x20,
    RGB332 = 0x30,
    RGB565 = 0x40,
    RGB888 = 0x50,
    XRGB8888 = 0x80,
    ARGB8888 = 0x81,
}

#[repr(transparent)]
#[derive(Debug, Serialize, Deserialize)]
pub struct Compression(u8);

bitflags! {
    impl Compression: u8{
        /// LZ4 lossless compression
        const LZ4 = 1 << 0;

        // The source may set any bits
        const _ = !0;
    }
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConnectorType {
    Panel = 0,
    VGA = 1,
    Composite = 2,
    SVideo = 3,
    Component = 4,
    DVI = 5,
    DisplayPort = 6,
    HDMI = 7,
}

#[repr(transparent)]
#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectorDescriptorFlags(u32);

bitflags! {
    impl ConnectorDescriptorFlags: u32 {
        /// Connector status can change (polled every 10 seconds)
        const POLL_STATUS = 1 << 0;
        /// Interlaced modes are supported
        const INTERLACE = 1 << 1;
        /// Doublescan modes are supported
        const DOUBLESCAN = 1 << 2;

        // The source may set any bits
        const _ = !0;
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ConnectorDescriptor {
    connector_type: ConnectorType,
    flags: ConnectorDescriptorFlags,
}

pub struct PixelDataEndpoint {
    ep_rx: EndpointReceiver,
    // A collection of the small buffers we've allocated for submission to AIO to read from the endpoint.
    ep_buf: Vec<BytesMut>,
    // The full contents of a transmitted buffer are copied here.
    buf: BytesMut,
    // If compression is enabled, the received buffer is decompressed here.
    compress_buf: BytesMut,
}

#[repr(transparent)]
#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayModeFlags(u32);

bitflags! {
    impl DisplayModeFlags: u32{
        // These flags are from DRM
        const PHSYNC = 1 << 0;
        const NHSYNC = 1 << 1;
        const PVSYNC = 1 << 2;
        const NVSYNC = 1 << 3;
        const INTERLACE = 1 << 4;
        const DBLSCAN = 1 << 5;
        const CSYNC = 1 << 6;
        const PCSYNC = 1 << 7;
        const NCSYNC = 1 << 8;
        const HSKEW = 1 << 9;
        const DBLCLK = 1 << 12;

        // These flags are only for GUD
        const PREFERRED = 1 << 10;

        // The source may set any bits
        const _ = !0;
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayMode {
    pub clock: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub flags: DisplayModeFlags,
}

impl DisplayMode {
    pub fn from_res(width: u32, height: u32, max_fps: f32) -> Self {
        // What maps to what was extracted from the `drm_mode_detailed` function in `drivers/gpu/drm/drm_edid.c` of the linux kernel.

        let hdisplay = width as u16;
        let vdisplay = height as u16;

        // TODO: find better values for these
        let hsync_offset = 0u16;
        let hsync_pulse_width = 1u16;
        let hblank = hsync_offset + hsync_pulse_width;

        let hsync_start = hdisplay + hsync_offset;
        let hsync_end = hsync_start + hsync_pulse_width;
        let htotal = hdisplay + hblank;

        assert!(hsync_pulse_width > 0, "Pulse width can't be 0");
        assert!(
            hsync_offset + hsync_pulse_width <= hblank,
            "htotal must be no less than hsync_end"
        );

        // TODO: find better values for these
        let vsync_offset = 0u16;
        let vsync_pulse_width = 1u16;
        let vblank = vsync_offset + vsync_pulse_width;

        let vsync_start = vdisplay + vsync_offset;
        let vsync_end = vsync_start + vsync_pulse_width;
        let vtotal = vdisplay + vblank;

        assert!(vsync_pulse_width > 0, "Pulse width can't be 0");
        assert!(
            vsync_offset + vsync_pulse_width <= vblank,
            "vtotal must be no less than vsync_end"
        );

        let clock: u32 = (max_fps * htotal as f32 * vtotal as f32 / 1000f32) as u32;

        DisplayMode {
            clock,
            hdisplay,
            vdisplay,
            hsync_start,
            hsync_end,
            htotal,
            vsync_start,
            vsync_end,
            vtotal,
            flags: DisplayModeFlags::empty(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SetBuffer {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub length: u32,
    pub compression: Compression,
    pub compressed_length: u32,
}

#[derive(Debug)]
pub enum Event<'a> {
    GetDescriptor(GetDescriptor<'a>),
    GetDisplayModes(GetDisplayModes<'a>),
    GetPixelFormats(GetPixelFormats<'a>),
    Buffer(SetBuffer),
}

#[derive(Debug)]
pub struct GetDescriptor<'a> {
    sender: CtrlSender<'a>,
}

#[derive(Debug)]
pub struct GetDisplayModes<'a> {
    sender: CtrlSender<'a>,
}

#[derive(Debug)]
pub struct GetPixelFormats<'a> {
    sender: CtrlSender<'a>,
}

impl<'a> GetDescriptor<'a> {
    pub fn send_descriptor(
        self,
        min_width: u32,
        min_height: u32,
        max_width: u32,
        max_height: u32,
    ) -> anyhow::Result<()> {
        let descriptor = DisplayDescriptor {
            magic: GUD_DISPLAY_MAGIC,
            version: 1,
            flags: DisplayDescriptorFlags::empty(),
            compression: Compression::LZ4,
            max_height,
            max_width,
            min_height,
            min_width,
            max_buffer_size: max_height * max_width * 4,
        };

        let mut buf: [u8; 30] = [0; 30];
        ssmarshal::serialize(&mut buf, &descriptor).context("serialize display descriptor")?;

        self.sender.send(&buf).context("send display descriptor")?;
        debug!("sent display descriptor {:?}", descriptor);
        Ok(())
    }
}

impl<'a> GetDisplayModes<'a> {
    pub fn send_modes(self, modes: &[DisplayMode]) -> anyhow::Result<()> {
        let size = 24 * modes.len();
        if size > self.sender.len() {
            // TODO: proper Err
            panic!("too many display modes provided");
        }

        let mut buf = vec![0; size];
        let mut pos = 0;
        for mode in modes {
            pos = pos + ssmarshal::serialize(&mut buf[pos..], mode).context("serialize mode")?;
        }

        self.sender.send(&buf).context("send modes")?;

        Ok(())
    }
}

impl<'a> GetPixelFormats<'a> {
    pub fn send_pixel_formats(self, formats: &[PixelFormat]) -> anyhow::Result<()> {
        let formats_u8: Vec<u8> = formats.iter().map(|f| f.clone() as u8).collect();
        self.sender
            .send(&formats_u8)
            .context("send pixel formats")?;
        debug!("sent pixel formats: {:?}", formats);
        Ok(())
    }
}

#[repr(transparent)]
#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayDescriptorFlags(u32);

bitflags! {
    impl DisplayDescriptorFlags: u32{
        /// Always do a status request after a SET request.
        /// This is used by the Linux gadget driver since it has no way to control
        /// the status stage of a control OUT request that has a payload.
        const STATUS_ON_SET = 1 << 0;
        /// Always send the entire framebuffer when flushing changes.
        /// The GUD_REQ_SET_BUFFER request will not be sent before each bulk transfer,
        /// it will only be sent if the previous bulk transfer had failed.
        /// This gives the device a chance to reset its state machine if needed.
        /// This flag can not be used in combination with compression.
        const FULL_UPDATE = 1 << 1;

        // The source may set any bits
        const _ = !0;
    }
}

#[derive(Debug, Serialize)]
struct DisplayDescriptor {
    magic: u32,
    version: u8,
    flags: DisplayDescriptorFlags,
    compression: Compression,
    max_buffer_size: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

pub fn event(event: custom::Event) -> anyhow::Result<Option<Event>> {
    match event {
        custom::Event::Enable => {}
        custom::Event::Bind => {}
        custom::Event::SetupDeviceToHost(req) => {
            let ctrl_req = req.ctrl_req();
            match ctrl_req.request {
                GUD_REQ_GET_STATUS => {
                    req.send(&[Status::Ok as u8]).context("send status")?;
                    debug!("sent status");
                }
                GUD_REQ_GET_DESCRIPTOR => {
                    return Ok(Some(Event::GetDescriptor(GetDescriptor { sender: req })));
                }
                GUD_REQ_GET_FORMATS => {
                    return Ok(Some(Event::GetPixelFormats(GetPixelFormats {
                        sender: req,
                    })));
                }
                GUD_REQ_GET_PROPERTIES => {
                    let sent = req
                        .send(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
                        .context("send properties")?;
                    debug!("sent properties {}", sent);
                }
                GUD_REQ_GET_CONNECTORS => {
                    let connectors = [ConnectorDescriptor {
                        connector_type: ConnectorType::Panel,
                        flags: ConnectorDescriptorFlags::empty(),
                    }];

                    let mut buf: [u8; 5] = [0; 5];
                    ssmarshal::serialize(&mut buf, &connectors).context("serialize connectors")?;
                    req.send(&buf).context("send connectors")?;
                    debug!("sent connectors");
                }
                GUD_REQ_GET_CONNECTOR_PROPERTIES => {
                    req.send(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
                        .context("send connector properties")?;
                    debug!("sent connector properties");
                }
                GUD_REQ_GET_CONNECTOR_MODES => {
                    return Ok(Some(Event::GetDisplayModes(GetDisplayModes {
                        sender: req,
                    })));
                }
                GUD_REQ_GET_CONNECTOR_EDID => {
                    req.send(&[0]).context("send EDIDs")?;
                    debug!("sent EDIDs");
                }
                GUD_REQ_GET_CONNECTOR_STATUS => {
                    req.send(&[ConnectorStatus::CONNECTED.bits()])
                        .context("send connector status")?;
                    debug!("sent connector status");
                }
                req => {
                    warn!("unhandled SetupDeviceToHost request {:x}", req);
                }
            }
        }
        custom::Event::SetupHostToDevice(req) => {
            let ctrl_req = req.ctrl_req();
            match ctrl_req.request {
                GUD_REQ_SET_CONNECTOR_FORCE_DETECT => {
                    debug!("connector set to {}", ctrl_req.value);
                    req.recv_all().context("recv set connector")?;
                }
                GUD_REQ_SET_STATE_CHECK => {
                    debug!("received state check");
                    req.recv_all().context("recv set state check")?;
                }
                GUD_REQ_SET_CONTROLLER_ENABLE => {
                    let req = req.recv_all().context("recv set controller enable")?;
                    debug!("received controller enable: {:?}", req);
                }
                GUD_REQ_SET_DISPLAY_ENABLE => {
                    let req = req.recv_all().context("recv set display enable")?;
                    debug!("received display enable: {:?}", req);
                }
                GUD_REQ_SET_STATE_COMMIT => {
                    req.recv_all().context("recv set state commit")?;
                    debug!("received state commit");
                }
                GUD_REQ_SET_BUFFER => {
                    let req = req.recv_all().context("recv set buffer")?;
                    let v: SetBuffer;
                    (v, _) =
                        ssmarshal::deserialize(req.as_slice()).context("deserialize set buffer")?;
                    debug!("received set buffer: {:?}", v);
                    return Ok(Some(Event::Buffer(v)));
                }
                v => {
                    warn!("unhandled set request {:x}", v);
                }
            }
        }
        event => {
            warn!("unhandled event {:?}", event);
        }
    }
    Ok(None)
}

impl PixelDataEndpoint {
    pub fn new() -> (Self, Endpoint) {
        let (ep_rx, ep_dir) = EndpointDirection::host_to_device();

        (
            Self {
                ep_rx,
                ep_buf: Vec::new(),
                buf: BytesMut::new(),
                compress_buf: BytesMut::new(),
            },
            Endpoint::bulk(ep_dir),
        )
    }

    pub fn recv_buffer(
        &mut self,
        info: SetBuffer,
        fb: &mut [u8],
        fb_pitch: usize,
        bpp: usize,
    ) -> anyhow::Result<()> {
        let start = Instant::now();
        let max_packet_size = self.ep_rx.max_packet_size().unwrap();

        let len = if !info.compression.is_empty() {
            info.compressed_length
        } else {
            info.length
        } as usize;
        self.buf.clear();

        // Ensure the buffer is large enough to fit all incoming data.
        if self.buf.capacity() < len {
            self.buf.reserve(len - self.buf.capacity());
        }

        // Read the incoming data fully into the buffer.
        let read_start = Instant::now();
        while self.buf.len() < len {
            let buf = self
                .ep_buf
                .pop()
                .unwrap_or_else(|| BytesMut::with_capacity(max_packet_size));
            let buf = self.ep_rx.recv(buf).context("read bulk ep")?;
            if buf.is_none() {
                continue;
            }
            let mut buf = buf.unwrap();
            self.buf.extend_from_slice(&buf);
            buf.clear();
            self.ep_buf.push(buf);
        }
        trace!("read buffer took {}ms", read_start.elapsed().as_millis());

        if self.buf.len() != len {
            // TODO: proper Err
            panic!("expected buf len {}, got {}", len, self.buf.len());
        }

        let buf = if !info.compression.is_empty() {
            let decompress_start = Instant::now();
            if self.compress_buf.len() < info.length as usize {
                self.compress_buf
                    .resize(info.length as usize - self.compress_buf.capacity(), 0);
            }
            lz4::block::decompress_to_buffer(
                &self.buf,
                Some(info.length as i32),
                &mut self.compress_buf,
            )
            .context("lz4 decompress")?;
            trace!(
                "decompress buffer took {}ms",
                decompress_start.elapsed().as_millis()
            );
            &self.compress_buf
        } else {
            &self.buf
        };

        let mut y = info.y as usize;
        let end_y = (info.y + info.height) as usize;

        let line_len = info.width as usize * bpp;
        let line_start = info.x as usize * bpp;

        let mut buf_pos = 0usize;
        while y < end_y {
            let fb_start = (y * fb_pitch) + line_start;
            let fb_end = fb_start + line_len;
            fb[fb_start..fb_end].copy_from_slice(&buf[buf_pos..buf_pos + line_len]);
            buf_pos += line_len;
            y += 1;
        }

        trace!("recv_buffer took {}ms", start.elapsed().as_millis());

        Ok(())
    }
}
