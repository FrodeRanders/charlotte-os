#include <stddef.h>
#include "flanterm.h"
#include "backends/fb.h"

size_t q3_base_size(void) { return sizeof(struct flanterm_context); }
size_t q3_base_alignment(void) { return _Alignof(struct flanterm_context); }
size_t q3_malloc_alignment(void) { return _Alignof(max_align_t); }
void q3_deinit(struct flanterm_context *ctx, void (*release)(void *, size_t)) {
    ctx->deinit(ctx, release);
}

#ifdef Q3_LAYOUT_PROBE
#include <stdio.h>
int main(void) {
    printf("%zu %zu\n", q3_malloc_alignment(), q3_base_alignment());
    return 0;
}
#endif
