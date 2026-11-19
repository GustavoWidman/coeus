# A push service for the VM test. It decrypts each message as the browser of
# RFC 8291's appendix A, verifies the VAPID token, logs what it saw as a JSON
# line, and answers 410 to an endpoint ending in /gone and 201 to the rest.
import base64
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import encode_dss_signature
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF


def b64(text):
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def point(raw):
    return ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), raw)


UA_PRIVATE = ec.derive_private_key(
    int.from_bytes(b64("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"), "big"),
    ec.SECP256R1(),
)
UA_PUBLIC = b64(
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"
)
AUTH = b64("BTBZMqHH6r4Tts7J_aSIgg")


def hkdf(salt, ikm, info, length):
    return HKDF(hashes.SHA256(), length, salt, info).derive(ikm)


def decrypt(body):
    salt, idlen = body[:16], body[20]
    sender = body[21:21 + idlen]
    shared = UA_PRIVATE.exchange(ec.ECDH(), point(sender))
    ikm = hkdf(AUTH, shared, b"WebPush: info\0" + UA_PUBLIC + sender, 32)
    key = hkdf(salt, ikm, b"Content-Encoding: aes128gcm\0", 16)
    nonce = hkdf(salt, ikm, b"Content-Encoding: nonce\0", 12)
    record = AESGCM(key).decrypt(nonce, body[21 + idlen:], None).rstrip(b"\0")
    assert record.endswith(b"\x02"), "not the last record"
    return record[:-1]


def verify(authorization):
    scheme, _, fields = authorization.partition(" ")
    assert scheme == "vapid", authorization
    fields = dict(field.strip().split("=", 1) for field in fields.split(","))
    header, claims, signature = fields["t"].split(".")
    raw = b64(signature)
    der = encode_dss_signature(
        int.from_bytes(raw[:32], "big"), int.from_bytes(raw[32:], "big")
    )
    point(b64(fields["k"])).verify(
        der, f"{header}.{claims}".encode(), ec.ECDSA(hashes.SHA256())
    )
    return fields["k"], json.loads(b64(claims))


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        key, claims = verify(self.headers["Authorization"])
        seen = {
            "path": self.path,
            "key": key,
            "claims": claims,
            "ttl": self.headers["TTL"],
            "topic": self.headers["Topic"],
            "urgency": self.headers["Urgency"],
            "message": json.loads(decrypt(body)),
        }
        with open(sys.argv[1], "a") as log:
            log.write(json.dumps(seen) + "\n")
        self.send_response(410 if self.path.endswith("/gone") else 201)
        self.send_header("Content-Length", "0")
        self.end_headers()


HTTPServer(("127.0.0.1", 8080), Handler).serve_forever()
