"""Session-local CPU-answer echo. No network, torch import, or planning."""
import sys
import time
from . import protocol

def outcome(request_id, kind, **payload):
    return {"schema_version": "planning-kernel-contract/0.1", "request_id": request_id, "frame_index": 0, "frame_type": kind, **payload}

def run(reader, writer):
    active = None
    while True:
        message = protocol.read(reader)
        if message is None:
            return
        if not isinstance(message, dict) or set(message) != {"kind", "payload"}:
            raise ValueError("invalid input envelope")
        kind, payload = message["kind"], message["payload"]
        if kind == "request":
            if active is not None or not isinstance(payload, dict) or payload.get("schema_version") != "planning-kernel-contract/0.1" or not payload.get("request_id"):
                raise ValueError("invalid request")
            active = payload["request_id"]
        elif kind == "reference":
            if active is None or not isinstance(payload, dict) or payload.get("request_id") != active:
                raise ValueError("reference identity mismatch")
            protocol.write(writer, outcome(active, "final_response", response=payload))
            active = None
        elif kind == "cancel":
            if active is None or payload != {"request_id": active}:
                raise ValueError("cancellation identity mismatch")
            protocol.write(writer, outcome(active, "cancelled"))
            active = None
        elif kind == "stage1":
            if active is not None or not isinstance(payload, dict):
                raise ValueError("invalid Stage 1 invocation")
            request_id = payload.get("request", {}).get("request_id")
            if not isinstance(request_id, str) or not request_id:
                raise ValueError("invalid Stage 1 identity")
            try:
                from .stage1 import compute
                result = compute(payload)
                import torch
                protocol.write(writer, {"kind":"stage1_result", "payload":{"profile":"stage1-atomic-v1", "request_id":request_id,"framework_version":str(torch.__version__),"result":result}})
            except Exception:
                protocol.write(writer, outcome(request_id, "engine_error", error="Stage 1 worker failed"))
        elif kind == "environment":
            if active is not None or payload != {}:
                raise ValueError("invalid environment probe")
            protocol.write(writer, {"kind": "environment", "payload": framework_environment()})
        elif kind == "test_wait":
            # Explicit test plumbing, not a semantic planning input.
            if payload != {"milliseconds": 200}:
                raise ValueError("invalid test wait")
            time.sleep(0.2)
        else:
            raise ValueError("unknown input kind")

def framework_environment():
    # Importability cannot be established from distribution metadata alone.
    # The owned, bounded child isolates broken native imports from Rust.
    try:
        import torch
        if torch.__version__ != "2.6.0+cpu":
            return {"importable": False, "version": str(torch.__version__)}
        torch.empty(0, device="cpu")
        return {"importable": True, "version": str(torch.__version__)}
    except Exception:
        return {"importable": False, "version": None}

def main():
    try:
        run(sys.stdin.buffer, sys.stdout.buffer)
    except (ValueError, EOFError) as error:
        print(str(error), file=sys.stderr)
        return 2
    return 0

if __name__ == "__main__":
    sys.exit(main())
