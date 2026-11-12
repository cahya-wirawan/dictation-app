/*
 * Narrow C shim over the pinned whisper.cpp API. Keeping the large
 * whisper_*_params structs on the C side means the Rust FFI never has to mirror
 * their layout, which changes between revisions. Only primitive types cross
 * the boundary.
 */
#include "whisper.h"
#include <stdbool.h>
#include <stddef.h>

static void dw_quiet_log(enum ggml_log_level level, const char *text, void *user_data) {
    (void)level;
    (void)text;
    (void)user_data;
}

/* whisper.cpp/ggml log to stderr by default; we suppress them so the worker's
 * stderr stays a small, redacted diagnostics channel. */
void dw_silence_logs(void) { whisper_log_set(dw_quiet_log, NULL); }


static void dw_noop_close(void *ctx) { (void)ctx; }

typedef size_t (*dw_read_fn)(void *ctx, void *output, size_t read_size);
typedef bool (*dw_eof_fn)(void *ctx);

/* Loads from caller-provided read callbacks, so the caller can hash exactly
 * the bytes whisper.cpp consumes from one already-open file descriptor
 * (no hash-then-reopen race). */
struct whisper_context *dw_init_from_loader(void *ctx, dw_read_fn read, dw_eof_fn eof, int use_gpu) {
    struct whisper_model_loader loader = {ctx, read, eof, dw_noop_close};
    struct whisper_context_params p = whisper_context_default_params();
    p.use_gpu = use_gpu != 0;
    return whisper_init_with_params(&loader, p);
}

/* Deterministic greedy transcription (docs/03 §5): no translate, no
 * cross-session context, no timestamps, no temperature fallback,
 * non-speech tokens suppressed. `language` is "auto" or an ISO code. */
int dw_transcribe(struct whisper_context *ctx, const float *pcm, int n_samples,
                  const char *language, int n_threads) {
    struct whisper_full_params p = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    p.n_threads = n_threads;
    p.translate = false;
    p.no_context = true;
    p.no_timestamps = true;
    p.single_segment = false;
    p.print_special = false;
    p.print_progress = false;
    p.print_realtime = false;
    p.print_timestamps = false;
    p.token_timestamps = false;
    p.language = language;
    p.detect_language = false;
    p.suppress_blank = true;
    p.suppress_nst = true;
    p.temperature = 0.0f;
    p.temperature_inc = 0.0f;
    return whisper_full(ctx, p, pcm, n_samples);
}
