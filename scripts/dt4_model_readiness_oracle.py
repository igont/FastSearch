#!/usr/bin/env python3
"""Independent four-role compute oracle for TS-DT4-02 readiness."""

import argparse
import gc
import hashlib
import importlib.metadata
import json
from pathlib import Path

import numpy as np
import onnxruntime as ort
import torch
from transformers import AutoModel, AutoModelForCausalLM, AutoTokenizer

REVISIONS = {
    "arctic-embed-l-v2": "ac6544c8a46e00af67e330e85a9028c66b8cfd9a",
    "multilingual-e5-large": "66076b8dc6e367337e3e90e6fb309fb0f3addaf6",
    "nomic-embed-text-v2-moe": "1066b6599d099fbb93dfcb64f9c37a7c9e503e85",
    "qwen3-reranker-0.6b": "e61197ed45024b0ed8a2d74b80b4d909f1255473",
}
NOMIC_CODE_REVISION = "7710840340a098cfb869c4f65e87cf2b1b70caca"
QWEN_PREFIX = '<|im_start|>system\nJudge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be "yes" or "no".<|im_end|>\n<|im_start|>user\n'
QWEN_SUFFIX = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def normalize(values: np.ndarray) -> np.ndarray:
    norms = np.linalg.norm(values, axis=1, keepdims=True)
    if np.any(norms == 0) or not np.isfinite(norms).all():
        raise RuntimeError("embedding norm is invalid")
    return values / norms


def onnx_probe(root: Path, model_file: str, query: str, documents: list[dict], query_prefix: str, document_prefix: str, max_length: int, pooling: str) -> dict:
    tokenizer = AutoTokenizer.from_pretrained(root, local_files_only=True)
    texts = [query_prefix + query] + [document_prefix + item["text"] for item in documents]
    encoded = tokenizer(texts, padding=True, truncation=True, max_length=max_length, return_tensors="np")
    session = ort.InferenceSession(str(root / model_file), providers=["CPUExecutionProvider"])
    inputs = {name: encoded[name].astype(np.int64) for name in ["input_ids", "attention_mask"]}
    hidden = session.run(None, inputs)[0]
    if pooling == "cls":
        pooled = hidden[:, 0, :]
    else:
        mask = encoded["attention_mask"][..., None].astype(np.float32)
        pooled = (hidden * mask).sum(axis=1) / mask.sum(axis=1)
    vectors = normalize(pooled.astype(np.float32))
    scores = vectors[1:] @ vectors[0]
    order = [documents[index]["id"] for index in sorted(range(len(documents)), key=lambda index: (-float(scores[index]), documents[index]["id"]))]
    return {
        "dimension": int(vectors.shape[1]),
        "query_norm": float(np.linalg.norm(vectors[0])),
        "query_components": [float(value) for value in vectors[0, :8]],
        "scores": {documents[index]["id"]: float(scores[index]) for index in range(len(documents))},
        "order": order,
    }


def nomic_probe(query: str, documents: list[dict]) -> dict:
    repository = "nomic-ai/nomic-embed-text-v2-moe"
    tokenizer = AutoTokenizer.from_pretrained(repository, revision=REVISIONS["nomic-embed-text-v2-moe"], local_files_only=True)
    model = AutoModel.from_pretrained(repository, revision=REVISIONS["nomic-embed-text-v2-moe"], code_revision=NOMIC_CODE_REVISION, trust_remote_code=True, local_files_only=True).eval()
    texts = ["search_query: " + query] + ["search_document: " + item["text"] for item in documents]
    encoded = tokenizer(texts, padding=True, truncation=True, max_length=512, return_tensors="pt")
    with torch.no_grad():
        hidden = model(**encoded).last_hidden_state
        mask = encoded["attention_mask"].unsqueeze(-1).to(hidden.dtype)
        vectors = torch.nn.functional.normalize((hidden * mask).sum(dim=1) / mask.sum(dim=1), p=2, dim=1).cpu().numpy()
    scores = vectors[1:] @ vectors[0]
    result = {
        "dimension": int(vectors.shape[1]),
        "query_norm": float(np.linalg.norm(vectors[0])),
        "query_components": [float(value) for value in vectors[0, :8]],
        "scores": {documents[index]["id"]: float(scores[index]) for index in range(len(documents))},
        "order": [documents[index]["id"] for index in sorted(range(len(documents)), key=lambda index: (-float(scores[index]), documents[index]["id"]))],
    }
    del model
    gc.collect()
    return result


def qwen_probe(root: Path, fixture: dict) -> dict:
    tokenizer = AutoTokenizer.from_pretrained(root, local_files_only=True, padding_side="left")
    model = AutoModelForCausalLM.from_pretrained(root, local_files_only=True, torch_dtype=torch.float32).eval()
    yes, no = tokenizer.encode("yes", add_special_tokens=False), tokenizer.encode("no", add_special_tokens=False)
    if yes != [9693] or no != [2152]:
        raise RuntimeError(f"unexpected yes/no tokens: {yes}, {no}")
    prefix = tokenizer.encode(QWEN_PREFIX, add_special_tokens=False)
    suffix = tokenizer.encode(QWEN_SUFFIX, add_special_tokens=False)
    scores = {}
    with torch.no_grad():
        for pair in fixture["pairs"]:
            body = f'<Instruct>: {fixture["instruction"]}\n<Query>: {pair["query"]}\n<Document>: {pair["document"]}'
            body_tokens = tokenizer.encode(body, add_special_tokens=False)[: 8192 - len(prefix) - len(suffix)]
            tokens = torch.tensor([prefix + body_tokens + suffix], dtype=torch.long)
            logits = model(input_ids=tokens).logits[0, -1, [9693, 2152]].float()
            if not torch.isfinite(logits).all():
                raise RuntimeError(f'non-finite Qwen logits for {pair["id"]}')
            scores[pair["id"]] = float(torch.softmax(logits, dim=0)[0])
    del model
    gc.collect()
    return {"yes_token": 9693, "no_token": 2152, "scores": scores, "order": sorted(scores, key=lambda item: (-scores[item], item))}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--requirements", type=Path, required=True)
    parser.add_argument("--arctic-root", type=Path, required=True)
    parser.add_argument("--e5-root", type=Path, required=True)
    parser.add_argument("--qwen-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    fixture = json.loads(args.cases.read_text(encoding="utf-8"))
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    identities = {item["slug"]: item["revision"] for item in manifest["models"]}
    if identities != REVISIONS:
        raise RuntimeError(f"model manifest does not match oracle identities: {identities}")
    embedding = fixture["embedding"]
    observations = {
        "arctic-embed-l-v2": onnx_probe(args.arctic_root, "onnx/model.onnx", embedding["query"], embedding["documents"], "query: ", "", 8192, "cls"),
        "multilingual-e5-large": onnx_probe(args.e5_root, "model.onnx", embedding["query"], embedding["documents"], "query: ", "passage: ", 512, "mean"),
        "nomic-embed-text-v2-moe": nomic_probe(embedding["query"], embedding["documents"]),
        "qwen3-reranker-0.6b": qwen_probe(args.qwen_root, fixture["reranker"]),
    }
    output = {
        "schema": 1,
        "engine": "CPython 3.13.14",
        "dependencies": {name: importlib.metadata.version(name) for name in ["numpy", "onnxruntime", "torch", "transformers", "tokenizers"]},
        "script_sha256": sha256(Path(__file__).resolve()),
        "requirements_sha256": sha256(args.requirements),
        "cases_sha256": sha256(args.cases),
        "model_manifest_sha256": sha256(args.manifest),
        "model_revisions": REVISIONS,
        "nomic_code_revision": NOMIC_CODE_REVISION,
        "tolerance": fixture["tolerance"],
        "observations": observations,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
