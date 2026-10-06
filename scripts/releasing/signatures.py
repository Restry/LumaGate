"""Tauri base64-wrapped minisign verification, including the trusted comment."""
import base64
import hashlib
from pathlib import Path


def verify(asset, signature, public_key):
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    try:
        public_lines = base64.b64decode(public_key.strip(), validate=True).decode().splitlines()
        key = base64.b64decode(public_lines[1], validate=True)
        lines = base64.b64decode(signature.strip(), validate=True).decode().splitlines()
        sig = base64.b64decode(lines[1], validate=True)
        global_sig = base64.b64decode(lines[3], validate=True)
        if len(key) != 42 or key[:2] != b"Ed" or len(sig) != 74 or sig[:2] != b"ED" or key[2:10] != sig[2:10] or not lines[2].startswith("trusted comment: "):
            raise ValueError("Invalid minisign key/signature envelope")
        verifier = Ed25519PublicKey.from_public_bytes(key[10:])
        with Path(asset).open("rb") as stream:
            digest = hashlib.file_digest(stream, "blake2b").digest()
        verifier.verify(sig[10:], digest)
        verifier.verify(global_sig, sig[10:] + lines[2][len("trusted comment: "):].encode())
    except (InvalidSignature, ValueError, IndexError, UnicodeError) as error:
        raise ValueError(f"Updater signature rejected: {Path(asset).name}") from error
