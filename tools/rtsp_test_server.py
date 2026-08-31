"""Minimal RTSP/RTP-MJPEG (RFC 2435) test server for benching the Kite native RTSP client.

Serves ONE client: OPTIONS/DESCRIBE/SETUP/PLAY over TCP, then streams synthetic MJPEG
frames via RTP/UDP (to the SETUP client_port) or TCP-interleaved — whichever the client
asked for. Every 10th frame has its two middle fragments swapped to exercise the client's
reorder window (UDP only).

Usage: python rtsp_test_server.py --port 8600 [--fps 15] [--seconds 30]
"""
import argparse
import random
import socket
import struct
import sys
import threading
import time

FRAME_PAYLOAD = bytes(random.Random(42).randrange(256) for _ in range(6000))
FRAGMENT = 1000

def build_packets(seq0, ts, shuffle):
    """RFC 2435 packets for one frame: 8-byte JPEG header + scan fragment."""
    pkts = []
    offset = 0
    seq = seq0
    n = (len(FRAME_PAYLOAD) + FRAGMENT - 1) // FRAGMENT
    for i in range(n):
        chunk = FRAME_PAYLOAD[offset:offset + FRAGMENT]
        marker = 1 if i == n - 1 else 0
        hdr = struct.pack('!BBHII', 0x80, (marker << 7) | 26, seq & 0xFFFF, ts, 0x1234)
        jpeg = struct.pack('!B', 0) + struct.pack('!I', offset)[1:] + bytes([1, 50, 640 // 8, 480 // 8])
        pkts.append(hdr + jpeg + chunk)
        offset += len(chunk)
        seq += 1
    if shuffle and len(pkts) >= 4:
        pkts[1], pkts[2] = pkts[2], pkts[1]
    return pkts, seq

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
    ap.add_argument('--fps', type=int, default=15)
    ap.add_argument('--seconds', type=int, default=30)
    args = ap.parse_args()

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(('127.0.0.1', args.port))
    srv.listen(1)
    print(f'listening on {args.port}', flush=True)
    conn, peer = srv.accept()
    print(f'client {peer}', flush=True)

    buf = b''
    mode = None          # 'udp' | 'tcp'
    client_rtp = None    # (ip, port) for UDP
    udp_sock = None
    streaming = threading.Event()
    stop = threading.Event()
    lock = threading.Lock()  # control-socket writes (responses vs interleaved data)

    def streamer():
        seq = 100
        ts = 0
        frames = 0
        deadline = time.time() + args.seconds
        interval = 1.0 / args.fps
        while not stop.is_set() and time.time() < deadline:
            pkts, seq = build_packets(seq, ts, shuffle=(mode == 'udp' and frames % 10 == 9))
            for p in pkts:
                if mode == 'udp':
                    udp_sock.sendto(p, client_rtp)
                else:
                    with lock:
                        try:
                            conn.sendall(b'$' + bytes([0]) + struct.pack('!H', len(p)) + p)
                        except OSError:
                            return
            ts += 90000 // args.fps
            frames += 1
            time.sleep(interval)
        print(f'streamed {frames} frames', flush=True)

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
        status = '200 OK'
        if method == 'DESCRIBE':
            body = ('v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=Test\r\nt=0 0\r\n'
                    'a=control:*\r\n'
                    'm=video 0 RTP/AVP 26\r\n'
                    'a=rtpmap:26 JPEG/90000\r\n'
                    'a=control:streamid=0\r\n')
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
                extra = (f'Transport: RTP/AVP;unicast;client_port={port}-{port+1};'
                         f'server_port={sp}-{sp+1};source=127.0.0.1\r\n'
                         'Session: 4242ABCD;timeout=60\r\n')
        elif method == 'PLAY':
            extra = 'Session: 4242ABCD\r\nRange: npt=0.000-\r\n'
        elif method == 'TEARDOWN':
            pass

        resp = f'RTSP/1.0 {status}\r\nCSeq: {cseq}\r\n{extra}'
        if body:
            resp += f'Content-Length: {len(body)}\r\n\r\n{body}'
        else:
            resp += '\r\n'
        with lock:
            conn.sendall(resp.encode())

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
