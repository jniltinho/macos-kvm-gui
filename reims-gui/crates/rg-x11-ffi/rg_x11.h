/* rg_x11.h: C ABI of reims-gui's rg-x11 (crates/rg-x11-ffi), for the Go app's internal/embed.
 * Written by hand; crates/rg-x11-ffi's tests check it declares exactly the exported functions.
 *
 * Codes: 0 ok, -1 error (message in err), -2 panic, -3 null argument. err, when not NULL, gets
 * the message (at most errlen bytes, NUL-terminated). Handles are not thread-safe.
 */
#ifndef RG_X11_H
#define RG_X11_H

#include <stddef.h>
#include <stdint.h>

typedef struct RgxEmbedder RgxEmbedder;
typedef struct RgxEmbedded RgxEmbedded;

int rgx_probe(const char *display);
int rgx_set_wm_class(const char *display, uint32_t window, const char *class_name, char *err, size_t errlen);
int rgx_embedder_new(const char *display, uint32_t parent, RgxEmbedder **out, char *err, size_t errlen);
void rgx_embedder_free(RgxEmbedder *e);
int rgx_find_window(const RgxEmbedder *e, uint32_t pid, int kind, uint32_t timeout_ms, uint32_t *out,
                    char *err, size_t errlen);
int rgx_embed(const RgxEmbedder *e, uint32_t child, RgxEmbedded **out, char *err, size_t errlen);
int rgx_set_rect(RgxEmbedded *h, int32_t x, int32_t y, uint32_t w, uint32_t hgt, char *err, size_t errlen);
int rgx_pump(RgxEmbedded *h, char *err, size_t errlen);
int rgx_hide(RgxEmbedded *h, char *err, size_t errlen);
int rgx_show(RgxEmbedded *h, char *err, size_t errlen);
int rgx_focus(RgxEmbedded *h, char *err, size_t errlen);
int rgx_unfocus(RgxEmbedded *h, char *err, size_t errlen);
int rgx_return_focus_on_app_click(RgxEmbedded *h, int *moved, char *err, size_t errlen);
int rgx_is_alive(const RgxEmbedded *h);
int rgx_release(RgxEmbedded *h, char *err, size_t errlen);

#endif
