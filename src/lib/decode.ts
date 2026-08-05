/**
 * Turns the byte stream from the server into text the terminal can show.
 *
 * xterm.js decodes its input as UTF-8 and silently discards any byte that
 * cannot be part of a valid sequence. NetHack's `IBMgraphics` symset sends raw
 * CP437 -- `0xCD` for a horizontal wall, `0xFA` for floor -- so on that setting
 * the whole map disappears: the bytes never reach a cell, and the overlay,
 * which reads cells back to decide what a glyph landed on, sees every terrain
 * square as blank.
 *
 * So decoding happens here instead. A byte that starts a valid UTF-8 sequence
 * is decoded as UTF-8, because plenty of servers do send it; anything else is a
 * CP437 byte and becomes the character that code page names. The two cannot be
 * confused: a real UTF-8 sequence is a lead byte followed by continuation
 * bytes, and CP437 map symbols are not.
 *
 * One byte in, one character out for the CP437 path. That matters beyond
 * looks: the overlay counts characters to work out how many cells a write
 * covered.
 *
 * A sequence is decoded only from the bytes it arrives with. Nothing is held
 * back for the next chunk, because the demuxer hands over a glyph's character
 * as an item of its own -- holding that byte would strand every wall on the
 * map until the next one happened along. The cost is that a UTF-8 character
 * split across two reads decodes as its CP437 bytes instead. That is rare, it
 * costs two odd characters rather than a blank map, and it cannot happen at
 * all on the symsets that send one byte per symbol.
 */

/** CP437's upper half, `0x80` first. Its lower half is ASCII. */
const CP437_HIGH =
  "ÇüéâäàåçêëèïîìÄÅ" +
  "ÉæÆôöòûùÿÖÜ¢£¥₧ƒ" +
  "áíóúñÑªº¿⌐¬½¼¡«»" +
  "░▒▓│┤╡╢╖╕╣║╗╝╜╛┐" +
  "└┴┬├─┼╞╟╚╔╩╦╠═╬╧" +
  "╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀" +
  "αßΓπΣσµτΦΘΩδ∞φε∩" +
  "≡±≥≤⌠⌡÷≈°∙·√ⁿ²■ ";

/** How many bytes follow a UTF-8 lead byte, or 0 if it is not one. */
function continuationCount(lead: number): number {
  if (lead >= 0xc2 && lead <= 0xdf) return 1;
  if (lead >= 0xe0 && lead <= 0xef) return 2;
  if (lead >= 0xf0 && lead <= 0xf4) return 3;
  // 0xc0 and 0xc1 would be overlong; 0x80..0xbf is a stray continuation byte.
  return 0;
}

const isContinuation = (b: number) => b >= 0x80 && b <= 0xbf;

/** Decodes one chunk of server bytes into text for the terminal. */
export function decodeStream(bytes: Uint8Array): string {
  let out = "";
  let i = 0;

  while (i < bytes.length) {
    const b = bytes[i];
    if (b < 0x80) {
      out += String.fromCharCode(b);
      i++;
      continue;
    }

    const need = continuationCount(b);
    const rest = need > 0 ? bytes.subarray(i + 1, i + 1 + need) : null;
    if (rest && rest.length === need && rest.every(isContinuation)) {
      let cp = b & (0x7f >> need);
      for (const r of rest) cp = (cp << 6) | (r & 0x3f);
      out += String.fromCodePoint(cp);
      i += need + 1;
    } else {
      out += CP437_HIGH[b - 0x80];
      i++;
    }
  }

  return out;
}
