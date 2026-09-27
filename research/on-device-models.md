# What can a model on the phone do?

First written 2026-09-27.

## What the sources show

**A 3B model on the device is close to other small models and far from large ones.** Apple's
on-device model has about 3 billion parameters with 2-bit quantization-aware training. It scores
67.85 on MMLU, against 66.37 for Qwen-2.5-3B and 75.10 for Qwen-3-4B. The report names "a bigger
gap to larger models such as GPT-4o"
([Apple, Apple Intelligence Foundation Language Models: Tech Report 2025, arXiv:2507.13575](https://arxiv.org/abs/2507.13575)).

**The model runs only on some devices.** Apple Intelligence, and with it the on-device model,
needs a supported iPhone, iPad or Mac ([Apple Support 121115](https://support.apple.com/en-us/121115)).

**Phones are slow for anything larger.** A measurement study found that only models under about
4B parameters ran on powerful phones, with latencies over 30 seconds where a cloud call took under
10 seconds ([Yan and Ding, Are We There Yet?, arXiv:2504.00002](https://arxiv.org/abs/2504.00002)).

## What the sources do not show

- How Apple's model performs on Sjel's own tasks, such as labelling a mail or a transaction. Not
  measured.
