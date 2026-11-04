#include <stddef.h>
#include "flanterm.h"
#include "backends/fb.h"

size_t q3_base_size(void) { return sizeof(struct flanterm_context); }
size_t q3_base_alignment(void) { return _Alignof(struct flanterm_context); }
size_t q3_malloc_alignment(void) { return _Alignof(max_align_t); }
size_t q3_columns(struct flanterm_context *ctx) { return ctx->cols; }
size_t q3_rows(struct flanterm_context *ctx) { return ctx->rows; }
size_t q3_scale(struct flanterm_context *ctx) {
    return ((struct flanterm_fb_context *)ctx)->font_scale_x;
}
void q3_deinit(struct flanterm_context *ctx, void (*release)(void *, size_t)) {
    ctx->deinit(ctx, release);
}

#ifdef Q3_LAYOUT_PROBE
#include <stdio.h>
int main(void) {
    printf("%zu %zu %zu %zu %zu\n", q3_malloc_alignment(), q3_base_alignment(),
        sizeof(struct flanterm_fb_context), sizeof(struct flanterm_fb_char),
        sizeof(struct flanterm_fb_queue_item));
    return 0;
}
#endif
