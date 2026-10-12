"""Regenerate the AES-256 encryption fixtures (`aes256-*.pdf`).

AES-256 is the standard security handler's revision 5 (an Adobe extension to PDF 1.7) and
revision 6 (ISO 32000-2): the file key is a random 32 bytes, stored encrypted under keys
derived from each password (`/UE`, `/OE`), and strings and streams are AES-256-CBC with the
file key itself (crypt filter `/AESV3`). Each fixture shows one line of text:

    aes256-r6-open.pdf      revision 6 by PyMuPDF; empty user password, owner password `owner`
    aes256-r6-user.pdf      revision 6 by PyMuPDF; user password `user`, owner password `owner`
    aes256-r5.pdf           revision 5, written here from the specification with fixed salts
                            and file key; user password `user`, owner password `owner`
    aes128-mupdf.pdf        revision 4, AES-128, by PyMuPDF; empty user password — like every
                            file MuPDF encrypts, its `/Encrypt` dictionary is written into the
                            trailer rather than referenced

The revision 6 files come from an independent writer (PyMuPDF/MuPDF), so reading them checks
the key derivation against someone else's. The revision 5 file is built here with `hashlib`
and `cryptography`, not with the code under test.

    python make_aes256_fixtures.py        # writes next to this script; needs pymupdf, cryptography

The revision 6 files are not byte-for-byte reproducible (MuPDF salts them at random); the
revision 5 file is.
"""

import hashlib
import struct
from pathlib import Path

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

HERE = Path(__file__).resolve().parent
TEXT = "Encrypted with AES-256"


def with_pymupdf(name: str, user: str, method: str = "PDF_ENCRYPT_AES_256") -> None:
    import pymupdf

    doc = pymupdf.open()
    page = doc.new_page(width=300, height=200)
    page.insert_text((40, 100), TEXT, fontsize=12)
    doc.save(
        HERE / name,
        encryption=getattr(pymupdf, method),
        owner_pw="owner",
        user_pw=user,
        permissions=pymupdf.PDF_PERM_PRINT,
    )


def aes256_cbc(key: bytes, iv: bytes, data: bytes) -> bytes:
    enc = Cipher(algorithms.AES(key), modes.CBC(iv)).encryptor()
    return enc.update(data) + enc.finalize()


def pkcs7(data: bytes) -> bytes:
    n = 16 - len(data) % 16
    return data + bytes([n]) * n


def r5() -> None:
    file_key = bytes(range(0x40, 0x60))
    user, owner = b"user", b"owner"
    u_valid, u_key, o_valid, o_key = b"UVALSALT", b"UKEYSALT", b"OVALSALT", b"OKEYSALT"

    u = hashlib.sha256(user + u_valid).digest() + u_valid + u_key
    ue = aes256_cbc(hashlib.sha256(user + u_key).digest(), bytes(16), file_key)
    o = hashlib.sha256(owner + o_valid + u).digest() + o_valid + o_key
    oe = aes256_cbc(hashlib.sha256(owner + o_key + u).digest(), bytes(16), file_key)
    p = -4
    perms_plain = struct.pack("<i", p) + b"\xff\xff\xff\xff" + b"Tadb" + b"\x00\x00\x00\x00"
    ecb = Cipher(algorithms.AES(file_key), modes.ECB()).encryptor()
    perms = ecb.update(perms_plain) + ecb.finalize()

    content = f"BT /F1 12 Tf 40 100 Td ({TEXT}) Tj ET".encode()
    iv = bytes(range(16))
    stream = iv + aes256_cbc(file_key, iv, pkcs7(content))

    def hexstr(b: bytes) -> bytes:
        return b"<" + b.hex().encode() + b">"

    objects = [
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 200]"
        b"/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
        b"<</Length %d>>\nstream\n" % len(stream) + stream + b"\nendstream",
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
        b"<</Filter/Standard/V 5/R 5/Length 256"
        b"/CF<</StdCF<</CFM/AESV3/AuthEvent/DocOpen/Length 32>>>>/StmF/StdCF/StrF/StdCF"
        b"/O " + hexstr(o) + b"/U " + hexstr(u) + b"/OE " + hexstr(oe) + b"/UE " + hexstr(ue)
        + b"/P %d/Perms " % p + hexstr(perms) + b">>",
    ]
    out = bytearray(b"%PDF-1.7\n")
    offsets = []
    for i, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for off in offsets:
        out += b"%010d 00000 n \n" % off
    out += (
        b"trailer\n<</Size %d/Root 1 0 R/Encrypt 6 0 R"
        b"/ID[<00112233445566778899aabbccddeeff><00112233445566778899aabbccddeeff>]>>\n"
        b"startxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)
    )
    (HERE / "aes256-r5.pdf").write_bytes(bytes(out))


if __name__ == "__main__":
    with_pymupdf("aes256-r6-open.pdf", "")
    with_pymupdf("aes256-r6-user.pdf", "user")
    with_pymupdf("aes128-mupdf.pdf", "", "PDF_ENCRYPT_AES_128")
    r5()
    print("wrote", ", ".join(sorted(p.name for p in HERE.glob("aes*.pdf"))))
