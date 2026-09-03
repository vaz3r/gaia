from __future__ import annotations

import logging

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

from .base import ModelBackend

logger = logging.getLogger(__name__)


class TransformersBackend(ModelBackend):
    def __init__(self, config: dict):
        model_cfg = config["model"]
        self.model_name = model_cfg["name_or_path"]
        self.generation_cfg = dict(config["generation"])
        self.chat_template_cfg = config.get("chat_template", {})

        logger.info("Loading tokenizer from %s", self.model_name)
        self.tokenizer = AutoTokenizer.from_pretrained(
            self.model_name,
            trust_remote_code=model_cfg.get("trust_remote_code", False),
        )

        logger.info("Loading model from %s", self.model_name)
        dtype_str = model_cfg.get("dtype", "auto")
        dtype = getattr(torch, dtype_str, None) if dtype_str != "auto" else "auto"

        self.model = AutoModelForCausalLM.from_pretrained(
            self.model_name,
            torch_dtype=dtype,
            device_map=model_cfg.get("device_map", "auto"),
            attn_implementation=model_cfg.get("attn_implementation"),
            trust_remote_code=model_cfg.get("trust_remote_code", False),
        )
        self.model.eval()
        logger.info("Model loaded: %s", self.model_name)

        import os
        n_threads = os.cpu_count() or 4
        torch.set_num_threads(n_threads)
        logger.info("Using %d CPU threads", n_threads)

    def generate(self, messages: list[dict]) -> str:
        kwargs = {}
        if "qwen3" in self.model_name.lower():
            kwargs["enable_thinking"] = self.chat_template_cfg.get(
                "enable_thinking", False
            )

        text = self.tokenizer.apply_chat_template(
            messages,
            tokenize=False,
            add_generation_prompt=True,
            **kwargs,
        )

        inputs = self.tokenizer([text], return_tensors="pt").to(self.model.device)

        gen_kwargs = {
            k: v
            for k, v in self.generation_cfg.items()
            if v is not None and k not in ("presence_penalty",)
        }
        gen_kwargs["pad_token_id"] = self.tokenizer.eos_token_id

        with torch.no_grad():
            outputs = self.model.generate(**inputs, **gen_kwargs)

        output_ids = outputs[0][len(inputs.input_ids[0]) :]
        response = self.tokenizer.decode(output_ids, skip_special_tokens=True)
        return response
