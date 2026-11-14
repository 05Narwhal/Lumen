"""A `models.json` entry for a file `onnx_export.export` wrote (`photos-models::manifest`)."""


def entry(model_id: str, role: str, file: str, exported: dict, licence: str, **extra) -> dict:
    return {
        "id": model_id,
        "role": role,
        "file": file,
        "bytes": exported["bytes"],
        "sha256": exported["sha256"],
        "licence": licence,
        **extra,
    }
