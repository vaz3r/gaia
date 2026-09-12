import net from 'net';
import crypto from 'crypto';

// Fast bencode decoder for dictionary prefixes
function findDictEnd(buf, start = 0) {
  if (buf[start] !== 0x64) return -1; // 'd'
  let i = start + 1;
  while (i < buf.length) {
    const b = buf[i];
    if (b === 0x65) { // 'e'
      return i + 1;
    } else if (b === 0x69) { // integer: i...e
      i++;
      while (i < buf.length && buf[i] !== 0x65) i++;
      if (i < buf.length) i++;
    } else if (b >= 0x30 && b <= 0x39) { // string: <len>:...
      let lenStr = '';
      while (i < buf.length && buf[i] !== 0x3a) { // ':'
        lenStr += String.fromCharCode(buf[i]);
        i++;
      }
      i++; // skip ':'
      const strLen = parseInt(lenStr, 10);
      i += strLen;
    } else if (b === 0x6c || b === 0x64) { // list or sub-dict
      const end = findDictEnd(buf, i);
      if (end === -1) return -1;
      i = end;
    } else {
      i++;
    }
  }
  return -1;
}

function parseBencodeDict(buf) {
  const str = buf.toString('latin1');
  const dict = {};
  const utMatch = str.match(/11:ut_metadatai([0-9]+)e/);
  if (utMatch) {
    dict.ut_metadata = parseInt(utMatch[1], 10);
  }
  const sizeMatch = str.match(/13:metadata_sizei([0-9]+)e/);
  if (sizeMatch) {
    dict.metadata_size = parseInt(sizeMatch[1], 10);
  }
  return dict;
}

/**
 * Fetch raw bencoded info dictionary directly from a peer via BEP 9 (ut_metadata)
 */
function fetchFromPeer(infohashHex, ip, port, timeoutMs = 2500) {
  return new Promise((resolve, reject) => {
    const socket = new net.Socket();
    let isDone = false;

    const cleanup = (err) => {
      if (!isDone) {
        isDone = true;
        socket.destroy();
        if (err) reject(err);
      }
    };

    socket.setTimeout(timeoutMs);
    socket.on('timeout', () => cleanup(new Error(`Timeout connecting to ${ip}:${port}`)));
    socket.on('error', (e) => cleanup(e));

    const infohashBuf = Buffer.from(infohashHex, 'hex');
    const peerId = Buffer.concat([Buffer.from('-GA0001-'), crypto.randomBytes(12)]);

    // Handshake: 19 + "BitTorrent protocol" + 8 reserved (byte 5 = 0x10) + 20 infohash + 20 peerId
    const reserved = Buffer.alloc(8);
    reserved[5] = 0x10; // BEP 10 extension bit
    const handshake = Buffer.concat([
      Buffer.from([19]),
      Buffer.from('BitTorrent protocol', 'utf8'),
      reserved,
      infohashBuf,
      peerId
    ]);

    let state = 'HANDSHAKE';
    let buffer = Buffer.alloc(0);
    let remoteUtMetadataId = null;
    let metadataSize = 0;
    let numPieces = 0;
    const piecesReceived = new Map();

    socket.connect(port, ip, () => {
      socket.write(handshake);
    });

    socket.on('data', (chunk) => {
      // Keep timeout alive on incoming traffic
      socket.setTimeout(timeoutMs);
      buffer = Buffer.concat([buffer, chunk]);

      if (state === 'HANDSHAKE') {
        if (buffer.length < 68) return;
        if (buffer.slice(1, 20).toString() !== 'BitTorrent protocol') {
          return cleanup(new Error('Invalid BT protocol'));
        }
        const hasExtensions = (buffer[25] & 0x10) !== 0;
        if (!hasExtensions) {
          return cleanup(new Error('Peer does not support extensions'));
        }
        buffer = buffer.slice(68);
        state = 'MESSAGES';

        // Send BEP 10 Extended Handshake: length (4) + ID 20 + ExtID 0 + bencoded payload
        const extPayload = Buffer.from('d1:md11:ut_metadatai1eee', 'utf8');
        const extMsg = Buffer.alloc(4 + 1 + 1 + extPayload.length);
        extMsg.writeUInt32BE(1 + 1 + extPayload.length, 0);
        extMsg[4] = 20; // Extended message
        extMsg[5] = 0;  // Extended handshake
        extPayload.copy(extMsg, 6);
        socket.write(extMsg);
      }

      if (state === 'MESSAGES') {
        while (buffer.length >= 4) {
          const msgLen = buffer.readUInt32BE(0);
          if (msgLen === 0) {
            // Keep-alive
            buffer = buffer.slice(4);
            continue;
          }
          if (buffer.length < 4 + msgLen) {
            // Need more data
            break;
          }

          const msg = buffer.slice(4, 4 + msgLen);
          buffer = buffer.slice(4 + msgLen);

          const msgId = msg[0];
          if (msgId === 20) { // Extended message
            const extId = msg[1];
            const extPayload = msg.slice(2);

            if (extId === 0) { // Extended Handshake from peer
              const parsed = parseBencodeDict(extPayload);
              if (!parsed.ut_metadata) {
                return cleanup(new Error('Peer lacks ut_metadata'));
              }
              remoteUtMetadataId = parsed.ut_metadata;
              metadataSize = parsed.metadata_size || 0;
              if (metadataSize <= 0 || metadataSize > 15 * 1024 * 1024) {
                return cleanup(new Error(`Invalid metadata size: ${metadataSize}`));
              }
              numPieces = Math.ceil(metadataSize / 16384);

              // Request all pieces
              for (let p = 0; p < numPieces; p++) {
                const reqPayload = Buffer.from(`d8:msg_typei0e5:piecei${p}ee`, 'utf8');
                const reqMsg = Buffer.alloc(4 + 1 + 1 + reqPayload.length);
                reqMsg.writeUInt32BE(1 + 1 + reqPayload.length, 0);
                reqMsg[4] = 20;
                reqMsg[5] = remoteUtMetadataId;
                reqPayload.copy(reqMsg, 6);
                socket.write(reqMsg);
              }
            } else if (extId === 1) { // ut_metadata response
              const dictEnd = findDictEnd(extPayload, 0);
              if (dictEnd !== -1) {
                const headerDictStr = extPayload.slice(0, dictEnd).toString('latin1');
                const pieceMatch = headerDictStr.match(/5:piecei([0-9]+)e/);
                const typeMatch = headerDictStr.match(/8:msg_typei([0-9]+)e/);
                if (typeMatch && typeMatch[1] === '1' && pieceMatch) {
                  const pieceIdx = parseInt(pieceMatch[1], 10);
                  const pieceData = extPayload.slice(dictEnd);
                  piecesReceived.set(pieceIdx, pieceData);

                  if (piecesReceived.size === numPieces) {
                    const fullInfo = Buffer.concat(
                      Array.from({ length: numPieces }, (_, i) => piecesReceived.get(i))
                    );
                    const hash = crypto.createHash('sha1').update(fullInfo).digest('hex');
                    if (hash.toLowerCase() === infohashHex.toLowerCase()) {
                      isDone = true;
                      socket.destroy();
                      resolve(fullInfo);
                      return;
                    } else {
                      return cleanup(new Error(`SHA-1 mismatch: expected ${infohashHex} got ${hash}`));
                    }
                  }
                }
              }
            }
          }
        }
      }
    });
  });
}

/**
 * Race multiple candidate peers in parallel to fetch metadata on the fly
 */
export async function fetchMetadataOnTheFly(infohashHex, peerAddrs, timeoutMs = 3000) {
  if (!peerAddrs || peerAddrs.length === 0) {
    throw new Error('No candidate peers provided');
  }
  const promises = peerAddrs.map((p) => {
    const [ip, port] = p.replace(/\/[0-9]+/, '').split(':');
    return fetchFromPeer(infohashHex, ip, parseInt(port, 10), timeoutMs);
  });
  return Promise.any(promises);
}

const DEFAULT_TRACKERS = [
  'udp://tracker.opentrackr.org:1337/announce',
  'udp://open.stealth.si:80/announce',
  'udp://tracker.torrent.eu.org:451/announce',
  'udp://explodie.org:6969/announce'
];

/**
 * Assembles a valid bencoded .torrent file in memory from the raw info dictionary buffer
 */
export function buildTorrentBuffer(rawInfoBuffer, primaryAnnounce = DEFAULT_TRACKERS[0], trackers = DEFAULT_TRACKERS) {
  const chunks = [];
  chunks.push(Buffer.from('d'));
  
  // 1. announce
  chunks.push(Buffer.from(`8:announce${Buffer.byteLength(primaryAnnounce)}:${primaryAnnounce}`));
  
  // 2. announce-list
  if (trackers && trackers.length > 0) {
    let listStr = '13:announce-listl';
    for (const tr of trackers) {
      listStr += `l${Buffer.byteLength(tr)}:${tr}e`;
    }
    listStr += 'e';
    chunks.push(Buffer.from(listStr));
  }
  
  // 3. comment
  const comment = 'GAIA Fast-Start On-The-Fly';
  chunks.push(Buffer.from(`7:comment${Buffer.byteLength(comment)}:${comment}`));
  
  // 4. created by
  const createdBy = 'GAIA Engine v1.0';
  chunks.push(Buffer.from(`10:created by${Buffer.byteLength(createdBy)}:${createdBy}`));
  
  // 5. creation date
  const nowSec = Math.floor(Date.now() / 1000);
  chunks.push(Buffer.from(`13:creation datei${nowSec}e`));
  
  // 6. info
  chunks.push(Buffer.from('4:info'));
  chunks.push(rawInfoBuffer);
  
  // End of dictionary
  chunks.push(Buffer.from('e'));
  
  return Buffer.concat(chunks);
}

/**
 * Builds a fast-start Turbo Magnet link with injected live peers and Tier-1 trackers
 */
export function buildTurboMagnet(infohashHex, name, peers = [], trackers = DEFAULT_TRACKERS) {
  const dn = name ? `&dn=${encodeURIComponent(name)}` : '';
  const trParams = trackers.map(t => `&tr=${encodeURIComponent(t)}`).join('');
  const peParams = (peers || []).slice(0, 5).map(p => `&x.pe=${encodeURIComponent(p.replace(/\/[0-9]+/, ''))}`).join('');
  return `magnet:?xt=urn:btih:${infohashHex.toLowerCase()}${dn}${trParams}${peParams}`;
}
