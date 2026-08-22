#!/usr/bin/env python3
"""Build the independent TS-DT4-01 ranking oracle."""

import argparse
import gc
import hashlib
import json
import os
from pathlib import Path

import numpy as np
import onnxruntime as ort
import torch
from transformers import AutoModel, AutoModelForCausalLM, AutoTokenizer

MODEL_REVISIONS = {
    "arctic": "ac6544c8a46e00af67e330e85a9028c66b8cfd9a",
    "e5": "66076b8dc6e367337e3e90e6fb309fb0f3addaf6",
    "nomic": "1066b6599d099fbb93dfcb64f9c37a7c9e503e85",
    "qwen": "e61197ed45024b0ed8a2d74b80b4d909f1255473",
}
NOMIC_CODE_REVISION = "7710840340a098cfb869c4f65e87cf2b1b70caca"
INSTRUCTION = "Given a web search query, retrieve relevant passages that answer the query"
QWEN_PREFIX = '<|im_start|>system\nJudge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be "yes" or "no".<|im_end|>\n<|im_start|>user\n'
QWEN_SUFFIX = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def normalized(vector: np.ndarray) -> np.ndarray:
    norm = np.linalg.norm(vector, axis=1, keepdims=True)
    if np.any(norm == 0) or not np.isfinite(norm).all():
        raise RuntimeError("embedding norm is invalid")
    return vector / norm


def read_records(root: Path) -> list[dict]:
    records = []
    for path in sorted((root / "corpus").glob("*.md")):
        text = path.read_text(encoding="utf-8").replace("\r\n", "\n")
        body = text.split("\n---\n", 1)[1]
        heading, content = body.strip().split("\n", 1)
        title = heading.removeprefix("# ").strip()
        content = content.strip()
        relative = f"corpus/{path.name}"
        stable_id = (
            f"named-root-v1:ts-dt4-01-docs:{relative}:markdown:"
            f"{len(title.encode('utf-8'))}:{title}"
        )
        records.append(
            {
                "stable_id": stable_id,
                "title": title,
                "normalized_path": relative,
                "content": content,
                "content_sha256": hashlib.sha256(content.encode("utf-8")).hexdigest(),
            }
        )
    if len(records) != 10:
        raise RuntimeError(f"TS-DT4-01 requires ten records, found {len(records)}")
    return records


def onnx_embeddings(
    model_root: Path,
    model_file: Path,
    texts: list[str],
    prefix: str,
    max_length: int,
    pooling: str,
) -> np.ndarray:
    tokenizer = AutoTokenizer.from_pretrained(model_root, local_files_only=True)
    encoded = tokenizer(
        [prefix + text for text in texts],
        padding=True,
        truncation=True,
        max_length=max_length,
        return_tensors="np",
    )
    session = ort.InferenceSession(str(model_file), providers=["CPUExecutionProvider"])
    inputs = {name: encoded[name].astype(np.int64) for name in ["input_ids", "attention_mask"]}
    outputs = session.run(None, inputs)
    hidden = outputs[0]
    if pooling == "cls":
        pooled = hidden[:, 0, :]
    else:
        mask = encoded["attention_mask"][..., None].astype(np.float32)
        pooled = (hidden * mask).sum(axis=1) / mask.sum(axis=1)
    return normalized(pooled.astype(np.float32))


def nomic_embeddings(texts: list[str], prefix: str) -> np.ndarray:
    repository = "nomic-ai/nomic-embed-text-v2-moe"
    revision = MODEL_REVISIONS["nomic"]
    tokenizer = AutoTokenizer.from_pretrained(repository, revision=revision)
    model = AutoModel.from_pretrained(
        repository,
        revision=revision,
        code_revision=NOMIC_CODE_REVISION,
        trust_remote_code=True,
    ).eval()
    encoded = tokenizer(
        [prefix + text for text in texts],
        padding=True,
        truncation=True,
        max_length=512,
        return_tensors="pt",
    )
    with torch.no_grad():
        hidden = model(**encoded).last_hidden_state
        mask = encoded["attention_mask"].unsqueeze(-1).to(hidden.dtype)
        pooled = (hidden * mask).sum(dim=1) / mask.sum(dim=1)
        output = torch.nn.functional.normalize(pooled, p=2, dim=1).cpu().numpy()
    del model
    gc.collect()
    return output


def ranks(
    query_vector: np.ndarray,
    document_vectors: np.ndarray,
    records: list[dict],
    limit: int,
) -> list[dict]:
    scores = document_vectors @ query_vector[0]
    order = sorted(range(len(records)), key=lambda index: (-float(scores[index]), records[index]["stable_id"]))
    return [
        {"stable_id": records[index]["stable_id"], "rank": rank}
        for rank, index in enumerate(order[:limit], start=1)
    ]


def qwen_scores(query: str, records: list[dict]) -> dict[str, float]:
    repository = "Qwen/Qwen3-Reranker-0.6B"
    revision = MODEL_REVISIONS["qwen"]
    tokenizer = AutoTokenizer.from_pretrained(repository, revision=revision, padding_side="left")
    model = AutoModelForCausalLM.from_pretrained(
        repository, revision=revision, torch_dtype=torch.float32
    ).eval()
    yes_token = tokenizer.encode("yes", add_special_tokens=False)
    no_token = tokenizer.encode("no", add_special_tokens=False)
    if yes_token != [9693] or no_token != [2152]:
        raise RuntimeError(f"unexpected Qwen yes/no tokens: {yes_token}, {no_token}")
    prefix = tokenizer.encode(QWEN_PREFIX, add_special_tokens=False)
    suffix = tokenizer.encode(QWEN_SUFFIX, add_special_tokens=False)
    result = {}
    with torch.no_grad():
        for record in records:
            body = (
                f"<Instruct>: {INSTRUCTION}\n<Query>: {query}\n"
                f"<Document>: {record['content']}"
            )
            body_tokens = tokenizer.encode(body, add_special_tokens=False)[
                : 8192 - len(prefix) - len(suffix)
            ]
            tokens = torch.tensor([prefix + body_tokens + suffix], dtype=torch.long)
            logits = model(input_ids=tokens).logits[0, -1, [9693, 2152]].float()
            result[record["stable_id"]] = float(torch.softmax(logits, dim=0)[0])
    return result


def model_roots() -> dict[str, Path]:
    local = Path(os.environ["LOCALAPPDATA"]) / "FastSearch" / "models"
    hub = Path.home() / ".cache" / "huggingface" / "hub"
    return {
        "arctic": local / "arctic-embed-l-v2" / "runtime" / "models--Snowflake--snowflake-arctic-embed-l-v2.0" / "snapshots" / MODEL_REVISIONS["arctic"],
        "e5": local / "multilingual-e5-large" / "runtime" / "models--Qdrant--multilingual-e5-large-onnx" / "snapshots" / MODEL_REVISIONS["e5"],
        "nomic": hub / "models--nomic-ai--nomic-embed-text-v2-moe" / "snapshots" / MODEL_REVISIONS["nomic"],
        "qwen": hub / "models--Qwen--Qwen3-Reranker-0.6B" / "snapshots" / MODEL_REVISIONS["qwen"],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    roots = model_roots()
    records = read_records(args.fixture_root)
    contract_path = args.fixture_root / "oracle-contract.json"
    contract = json.loads(contract_path.read_text(encoding="utf-8"))
    model_count = int(contract["embedding_model_count"])
    candidates_per_model = int(contract["candidates_per_model"])
    candidate_slots = int(contract["candidate_slots_before_deduplication"])
    public_result_limit = int(contract["public_result_limit"])
    if model_count != 3 or candidate_slots != model_count * candidates_per_model:
        raise RuntimeError(f"invalid TS-DT4-01 candidate contract: {contract}")
    query = (args.fixture_root / "query.txt").read_text(encoding="utf-8").strip()
    texts = [record["content"] for record in records]

    arctic_query = onnx_embeddings(roots["arctic"], roots["arctic"] / "onnx/model.onnx", [query], "query: ", 8192, "cls")
    arctic_documents = onnx_embeddings(roots["arctic"], roots["arctic"] / "onnx/model.onnx", texts, "", 8192, "cls")
    e5_query = onnx_embeddings(roots["e5"], roots["e5"] / "model.onnx", [query], "query: ", 512, "mean")
    e5_documents = onnx_embeddings(roots["e5"], roots["e5"] / "model.onnx", texts, "passage: ", 512, "mean")
    nomic_query = nomic_embeddings([query], "search_query: ")
    nomic_documents = nomic_embeddings(texts, "search_document: ")
    candidates = {
        "arctic-embed-l-v2": ranks(arctic_query, arctic_documents, records, candidates_per_model),
        "multilingual-e5-large": ranks(e5_query, e5_documents, records, candidates_per_model),
        "nomic-embed-text-v2-moe": ranks(nomic_query, nomic_documents, records, candidates_per_model),
    }
    slots_before_deduplication = [
        {"model_slug": model_slug, **candidate}
        for model_slug, model_candidates in candidates.items()
        for candidate in model_candidates
    ]
    if len(slots_before_deduplication) != candidate_slots:
        raise RuntimeError(
            f"expected {candidate_slots} candidate slots, found {len(slots_before_deduplication)}"
        )
    candidate_ids = {
        item["stable_id"]
        for model_candidates in candidates.values()
        for item in model_candidates
    }
    selected = [record for record in records if record["stable_id"] in candidate_ids]
    probabilities = qwen_scores(query, selected)
    final_records = sorted(
        selected,
        key=lambda record: (-probabilities[record["stable_id"]], record["stable_id"]),
    )[:public_result_limit]
    final = [
        {
            "rank": rank,
            "stable_id": record["stable_id"],
            "title": record["title"],
            "normalized_path": record["normalized_path"],
            "project_scope": "general",
            "document_status": "actual",
            "content_sha256": record["content_sha256"],
        }
        for rank, record in enumerate(final_records, start=1)
    ]
    script_path = Path(__file__).resolve()
    lock_path = args.fixture_root.parents[1] / "oracle" / "requirements.lock"
    output = {
        "schema": 1,
        "engine": "CPython 3.13.14",
        "script_sha256": sha256(script_path),
        "requirements_sha256": sha256(lock_path),
        "corpus_manifest_sha256": sha256(args.fixture_root / "corpus-manifest.json"),
        "model_manifest_sha256": sha256(args.fixture_root / "model-manifest.json"),
        "query_sha256": sha256(args.fixture_root / "query.txt"),
        "oracle_contract_sha256": sha256(contract_path),
        "model_revisions": MODEL_REVISIONS,
        "nomic_code_revision": NOMIC_CODE_REVISION,
        "candidate_contract": contract,
        "embedding_candidates": candidates,
        "candidate_slots_before_deduplication": slots_before_deduplication,
        "deduplicated_stable_ids": sorted(candidate_ids),
        "qwen_probabilities": probabilities,
        "results": final,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8", newline="\n") as destination:
        destination.write(json.dumps(output, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
