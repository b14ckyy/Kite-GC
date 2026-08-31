"""Minimal RTSP test server for benching the Kite native RTSP client.

Serves ONE client (OPTIONS/DESCRIBE/SETUP/PLAY over TCP), then streams RTP via UDP (to the
SETUP client_port) or TCP-interleaved — whichever the client asked for. Three payloads:

  --codec mjpeg              synthetic RFC 2435 MJPEG frames (default; no file needed).
                             Every 10th frame has two fragments swapped (UDP only) to
                             exercise the client's reorder window.
  --codec h264 --file X.264  RFC 6184: an Annex-B file WITH access-unit delimiters
                             (generate: ffmpeg ... -bf 0 -bsf:v h264_metadata=aud=insert
                             -f h264 X.264). Single-NAL + FU-A packetization, sprop
                             parameter sets in the SDP, AUs looped at --fps.
  --codec h265 --file X.265  RFC 7798, same idea (hevc_metadata=aud=insert), FU type 49,
                             sprop-vps/sps/pps.

Usage: python rtsp_test_server.py --port 8600 [--codec mjpeg|h264|h265] [--file PATH]
       [--fps 15] [--seconds 30]
"""
import argparse
import base64
import random
import socket
import struct
import threading
import time

MTU = 1200
FRAME_PAYLOAD = bytes(random.Random(42).randrange(256) for _ in range(6000))
FRAGMENT = 1000


def rtp_header(marker, pt, seq, ts):
    return struct.pack('!BBHII', 0x80, ((1 if marker else 0) << 7) | pt, seq & 0xFFFF,
                       ts & 0xFFFFFFFF, 0x1234)


# ── MJPEG (RFC 2435) ─────────────────────────────────────────────────────────

def mjpeg_packets(seq, ts, shuffle):
    pkts = []
    offset = 0
    n = (len(FRAME_PAYLOAD) + FRAGMENT - 1) // FRAGMENT
    for i in range(n):
        chunk = FRAME_PAYLOAD[offset:offset + FRAGMENT]
        jpeg = struct.pack('!B', 0) + struct.pack('!I', offset)[1:] + bytes([1, 50, 640 // 8, 480 // 8])
        pkts.append(rtp_header(i == n - 1, 26, seq, ts) + jpeg + chunk)
        offset += len(chunk)
        seq += 1
    if shuffle and len(pkts) >= 4:
        pkts[1], pkts[2] = pkts[2], pkts[1]
    return pkts, seq


# ── Annex-B parsing (H264/H265 files) ────────────────────────────────────────

def parse_annexb(data):
    """Split an Annex-B stream into NAL units (start codes removed)."""
    starts = []
    j = 0
    while True:
        j = data.find(b'\x00\x00\x01', j)
        if j < 0:
            break
        starts.append(j + 3)
        j += 3
    nals = []
    for k, s in enumerate(starts):
        end = (starts[k + 1] - 3) if k + 1 < len(starts) else len(data)
        if k + 1 < len(starts) and end > s and data[end - 1] == 0:
            end -= 1  # 4-byte start code: its leading zero belongs to the separator
        if end > s:
            nals.append(data[s:end])
    return nals


def split_aus(nals, is_aud):
    """Group NALs into access units at the AUD boundaries."""
    aus, cur = [], []
    for nal in nals:
        if is_aud(nal):
            if cur:
                aus.append(cur)
            cur = [nal]
        else:
            cur.append(nal)
    if cur:
        aus.append(cur)
    return aus


# ── H264 (RFC 6184) ──────────────────────────────────────────────────────────

def h264_type(nal):
    return nal[0] & 0x1F


def h264_packets(au, seq, ts):
    pkts = []
    for n, nal in enumerate(au):
        last_nal = n == len(au) - 1
        if len(nal) <= MTU:
            pkts.append((rtp_header(last_nal, 96, seq, ts) + nal))
            seq += 1
        else:
            ind = (nal[0] & 0xE0) | 28
            ntype = nal[0] & 0x1F
            body = nal[1:]
            off = 0
            while off < len(body):
                chunk = body[off:off + MTU]
                s = off == 0
                e = off + len(chunk) >= len(body)
                fu = bytes([ind, (0x80 if s else 0) | (0x40 if e else 0) | ntype]) + chunk
                pkts.append(rtp_header(last_nal and e, 96, seq, ts) + fu)
                seq += 1
                off += len(chunk)
    return pkts, seq


def h264_fmtp(nals):
    sps = next((n for n in nals if h264_type(n) == 7), None)
    pps = next((n for n in nals if h264_type(n) == 8), None)
    if sps and pps:
        return ('packetization-mode=1;sprop-parameter-sets='
                f'{base64.b64encode(sps).decode()},{base64.b64encode(pps).decode()}')
    return 'packetization-mode=1'


# ── H265 (RFC 7798) ──────────────────────────────────────────────────────────

def h265_type(nal):
    return (nal[0] >> 1) & 0x3F


def h265_packets(au, seq, ts):
    pkts = []
    for n, nal in enumerate(au):
        last_nal = n == len(au) - 1
        if len(nal) <= MTU:
            pkts.append(rtp_header(last_nal, 96, seq, ts) + nal)
            seq += 1
        else:
            ind = bytes([(nal[0] & 0x81) | (49 << 1), nal[1]])
            ntype = h265_type(nal)
            body = nal[2:]
            off = 0
            while off < len(body):
                chunk = body[off:off + MTU]
                s = off == 0
                e = off + len(chunk) >= len(body)
                fu = ind + bytes([(0x80 if s else 0) | (0x40 if e else 0) | ntype]) + chunk
                pkts.append(rtp_header(last_nal and e, 96, seq, ts) + fu)
                seq += 1
                off += len(chunk)
    return pkts, seq


def h265_fmtp(nals):
    parts = []
    for key, ty in (('sprop-vps', 32), ('sprop-sps', 33), ('sprop-pps', 34)):
        nal = next((n for n in nals if h265_type(n) == ty), None)
        if nal:
            parts.append(f'{key}={base64.b64encode(nal).decode()}')
    return ';'.join(parts) if parts else None


# ── Server ───────────────────────────────────────────────────────────────────

def recv_request(conn, buf):
    while b'\r\n\r\n' not in buf:
        data = conn.recv(4096)
        if not data:
            return None, buf
        buf += data
    head, _, rest = buf.partition(b'\r\n\r\n')
    return head.decode('utf-8', 'replace'), rest


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, required=True)
    ap.add_argument('--codec', choices=['mjpeg', 'h264', 'h265'], default='mjpeg')
    ap.add_argument('--file', help='Annex-B elementary stream with AUDs (h264/h265)')
    ap.add_argument('--fps', type=int, default=15)
    ap.add_argument('--seconds', type=int, default=30)
    args = ap.parse_args()

    aus = None
    fmtp = None
    if args.codec != 'mjpeg':
        with open(args.file, 'rb') as f:
            nals = parse_annexb(f.read())
        if args.codec == 'h264':
            aus = split_aus(nals, lambda n: h264_type(n) == 9)
            fmtp = h264_fmtp(nals)
            rtpmap = 'H264/90000'
        else:
            aus = split_aus(nals, lambda n: h265_type(n) == 35)
            fmtp = h265_fmtp(nals)
            rtpmap = 'H265/90000'
        print(f'{args.codec}: {len(nals)} NALs, {len(aus)} access units', flush=True)
        media = f'm=video 0 RTP/AVP 96\r\na=rtpmap:96 {rtpmap}\r\n'
        if fmtp:
            media += f'a=fmtp:96 {fmtp}\r\n'
    else:
        media = 'm=video 0 RTP/AVP 26\r\na=rtpmap:26 JPEG/90000\r\n'

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(('127.0.0.1', args.port))
    srv.listen(1)
    print(f'listening on {args.port}', flush=True)
    conn, peer = srv.accept()
    print(f'client {peer}', flush=True)

    buf = b''
    mode = None
    client_rtp = None
    udp_sock = None
    streaming = threading.Event()
    stop = threading.Event()
    lock = threading.Lock()

    def send_pkt(p):
        if mode == 'udp':
            udp_sock.sendto(p, client_rtp)
        else:
            with lock:
                conn.sendall(b'$' + bytes([0]) + struct.pack('!H', len(p)) + p)

    def streamer():
        seq = 100
        ts = 0
        frames = 0
        deadline = time.time() + args.seconds
        interval = 1.0 / args.fps
        tick = 90000 // args.fps
        try:
            while not stop.is_set() and time.time() < deadline:
                if args.codec == 'mjpeg':
                    pkts, seq = mjpeg_packets(seq, ts, shuffle=(mode == 'udp' and frames % 10 == 9))
                elif args.codec == 'h264':
                    pkts, seq = h264_packets(aus[frames % len(aus)], seq, ts)
                else:
                    pkts, seq = h265_packets(aus[frames % len(aus)], seq, ts)
                for p in pkts:
                    send_pkt(p)
                ts += tick
                frames += 1
                time.sleep(interval)
        except OSError:
            pass
        print(f'streamed {frames} frames/AUs', flush=True)

    while True:
        req, buf = recv_request(conn, buf)
        if req is None:
            break
        line0 = req.splitlines()[0]
        method = line0.split(' ')[0]
        cseq = '0'
        transport = ''
        for l in req.splitlines()[1:]:
            k, _, v = l.partition(':')
            if k.strip().lower() == 'cseq':
                cseq = v.strip()
            if k.strip().lower() == 'transport':
                transport = v.strip()
        print(f'<{method} (CSeq {cseq})', flush=True)

        extra = ''
        body = ''
        if method == 'DESCRIBE':
            body = ('v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=Test\r\nt=0 0\r\n'
                    'a=control:*\r\n' + media + 'a=control:streamid=0\r\n')
            extra = (f'Content-Base: rtsp://127.0.0.1:{args.port}/test/\r\n'
                     'Content-Type: application/sdp\r\n')
        elif method == 'OPTIONS':
            extra = 'Public: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN, GET_PARAMETER\r\n'
        elif method == 'SETUP':
            if 'interleaved' in transport:
                mode = 'tcp'
                extra = ('Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n'
                         'Session: 4242ABCD;timeout=60\r\n')
            else:
                mode = 'udp'
                port = None
                for tok in transport.split(';'):
                    if tok.startswith('client_port='):
                        port = int(tok.split('=')[1].split('-')[0])
                client_rtp = (peer[0], port)
                udp_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                udp_sock.bind(('127.0.0.1', 0))
                sp = udp_sock.getsockname()[1]
                extra = (f'Transport: RTP/AVP;unicast;client_port={port}-{port + 1};'
                         f'server_port={sp}-{sp + 1};source=127.0.0.1\r\n'
                         'Session: 4242ABCD;timeout=60\r\n')
        elif method == 'PLAY':
            extra = 'Session: 4242ABCD\r\nRange: npt=0.000-\r\n'

        resp = f'RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n{extra}'
        if body:
            resp += f'Content-Length: {len(body)}\r\n\r\n{body}'
        else:
            resp += '\r\n'
        with lock:
            try:
                conn.sendall(resp.encode())
            except OSError:
                break

        if method == 'PLAY' and not streaming.is_set():
            streaming.set()
            threading.Thread(target=streamer, daemon=True).start()
        if method == 'TEARDOWN':
            break

    stop.set()
    time.sleep(0.3)
    conn.close()
    srv.close()
    print('done', flush=True)


if __name__ == '__main__':
    main()
