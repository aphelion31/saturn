use std::error::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use yrs::{
    Doc, ReadTxn, StateVector, Transact, Update, Xml, XmlElementPrelim, XmlFragment, XmlTextPrelim,
    updates::{decoder::Decode, encoder::Encode},
};

fn var(mut n: usize, out: &mut Vec<u8>) {
    while n >= 128 {
        out.push((n as u8) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn buf(b: &[u8], out: &mut Vec<u8>) {
    var(b.len(), out);
    out.extend_from_slice(b);
}
fn frame(name: &str, typ: usize, sub: usize, data: &[u8]) -> Vec<u8> {
    let mut v = vec![];
    buf(name.as_bytes(), &mut v);
    var(typ, &mut v);
    var(sub, &mut v);
    buf(data, &mut v);
    v
}
fn read_var(b: &[u8], p: &mut usize) -> usize {
    let mut n = 0;
    let mut shift = 0;
    loop {
        let c = b[*p];
        *p += 1;
        n |= ((c & 127) as usize) << shift;
        if c & 128 == 0 {
            return n;
        }
        shift += 7;
    }
}
fn read_buf<'a>(b: &'a [u8], p: &mut usize) -> &'a [u8] {
    let n = read_var(b, p);
    let v = &b[*p..*p + n];
    *p += n;
    v
}
async fn send(s: &mut tokio::net::TcpStream, b: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut h = vec![0x82];
    if b.len() < 126 {
        h.push(0x80 | b.len() as u8);
    } else {
        h.push(0xfe);
        h.extend_from_slice(&(b.len() as u16).to_be_bytes());
    }
    let mask = [1, 2, 3, 4];
    h.extend(mask);
    h.extend(b.iter().enumerate().map(|(i, x)| x ^ mask[i % 4]));
    s.write_all(&h).await?;
    Ok(())
}
async fn recv(s: &mut tokio::net::TcpStream) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut h = [0; 2];
    s.read_exact(&mut h).await?;
    let mut n = (h[1] & 127) as usize;
    if n == 126 {
        let mut x = [0; 2];
        s.read_exact(&mut x).await?;
        n = u16::from_be_bytes(x) as usize;
    } else if n == 127 {
        let mut x = [0; 8];
        s.read_exact(&mut x).await?;
        n = u64::from_be_bytes(x) as usize;
    }
    let mut b = vec![0; n];
    s.read_exact(&mut b).await?;
    Ok(b)
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        return Err("usage: live_ws_probe BASE SLUG PROJECT PAGE COOKIE".into());
    }
    let url = url::Url::parse(&args[1])?;
    if url.scheme() != "http" {
        return Err("probe requires http BASE".into());
    }
    let host = url.host_str().ok_or("missing host")?;
    let port = url.port_or_known_default().ok_or("missing port")?;
    let mut s = tokio::net::TcpStream::connect((host, port)).await?;
    let req = format!(
        "GET /live/collaboration?workspaceSlug={}&projectId={}&documentType=page HTTP/1.1\r\nHost: {}:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nCookie: {}\r\n\r\n",
        args[2], args[3], host, port, args[5]
    );
    s.write_all(req.as_bytes()).await?;
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0];
        s.read_exact(&mut b).await?;
        head.push(b[0]);
        if head.len() > 16384 {
            return Err("oversized handshake".into());
        }
    }
    if !head.starts_with(b"HTTP/1.1 101") {
        return Err(String::from_utf8_lossy(&head).into_owned().into());
    }
    let name = &args[4];
    send(&mut s, &frame(name, 2, 0, b"")).await?;
    let _ = recv(&mut s).await?;
    let doc = Doc::new();
    send(&mut s, &frame(name, 0, 0, &StateVector::default().encode_v1())).await?;
    let msg = recv(&mut s).await?;
    let mut p = 0;
    let _ = read_buf(&msg, &mut p);
    if read_var(&msg, &mut p) != 0 || read_var(&msg, &mut p) != 1 {
        return Err("unexpected sync response".into());
    }
    let data = read_buf(&msg, &mut p);
    doc.transact_mut().apply_update(Update::decode_v1(data)?)?;
    let vector = doc.transact().state_vector();
    let root = doc.get_or_insert_xml_fragment("default");
    let title = doc.get_or_insert_xml_fragment("title");
    {
        let mut t = doc.transact_mut();
        let len = root.len(&t);
        root.remove_range(&mut t, 0, len);
        let el = root.insert(&mut t, 0, XmlElementPrelim::empty("paragraph"));
        el.insert(&mut t, 0, XmlTextPrelim::new("Live smoke content"));
        let len = title.len(&t);
        title.remove_range(&mut t, 0, len);
        let el = title.insert(&mut t, 0, XmlElementPrelim::empty("heading"));
        el.insert_attribute(&mut t, "level", 1i64);
        el.insert(&mut t, 0, XmlTextPrelim::new("Live smoke title"));
    }
    let update = doc.transact().encode_state_as_update_v1(&vector);
    send(&mut s, &frame(name, 0, 2, &update)).await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    Ok(())
}
