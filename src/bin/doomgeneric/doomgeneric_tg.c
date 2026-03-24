#include "doomgeneric.h"
#include "doomkeys.h"
#include "m_controls.h"

#include <stdint.h>
#include <stddef.h>

typedef struct {
    uint8_t *ptr;
    uintptr_t len;
    uintptr_t width;
    uintptr_t height;
} TgFramebufferInfo;

extern int32_t tg_set_input_mode_polling(void);
extern int32_t tg_framebuffer_info(TgFramebufferInfo *out);
extern int32_t tg_framebuffer_flush(void);
extern uint32_t tg_get_ticks_ms(void);
extern void tg_sleep_ms(uint32_t ms);
extern int32_t tg_getchar_poll(void);

static uint8_t *g_fb = 0;
static size_t g_fb_len = 0;
static size_t g_w = 0;
static size_t g_h = 0;
static int32_t g_x = 0;
static int32_t g_y = 0;

static unsigned short g_key_queue[64];
static unsigned int g_key_write = 0;
static unsigned int g_key_read = 0;
// 按键保持状态：当前认为正在按下的 Doom key（-1 表示无按下状态）
static int g_pending_key = -1;
// 如果 g_pending_key 为空闲，则不会触发 release
static uint32_t g_release_deadline = 0;

static uint8_t map_ascii_to_doom(uint8_t ch) {
    switch (ch) {
    case 'q':
    case 'Q':
        return KEY_ESCAPE;
    case '\n':
    case '\r':
        return KEY_ENTER;
    default:
        return ch;
    }
}

static void push_key(int pressed, uint8_t doom_key) {
    unsigned short key_data = (unsigned short)((pressed << 8) | doom_key);
    g_key_queue[g_key_write] = key_data;
    g_key_write = (g_key_write + 1u) % 64u;
}

static void poll_keys_once(void) {
    uint32_t now = tg_get_ticks_ms();
    int32_t raw = tg_getchar_poll();

    if (raw >= 0 && raw <= 255) {
        uint8_t key = map_ascii_to_doom((uint8_t)raw);
        // 仍是相同按键：延长按住状态（避免松开）
        if (g_pending_key == (int)key) {
            g_release_deadline = now + 100;
            return;
        }

        // 新按键，先释放旧按键
        if (g_pending_key >= 0) {
            push_key(0, (uint8_t)g_pending_key);
        }

        // 报 Press 事件并进入按住态
        push_key(1, key);
        g_pending_key = (int)key;
        g_release_deadline = now + 100;
        return;
    }

    // 无输入时，到 deadline 才释放按键
    if (g_pending_key >= 0 && (int32_t)(now - g_release_deadline) >= 0) {
        push_key(0, (uint8_t)g_pending_key);
        g_pending_key = -1;
    }
}

static void put_px(size_t x, size_t y, uint32_t color) {
    if (g_fb == 0 || x >= g_w || y >= g_h) {
        return;
    }
    size_t idx = (y * g_w + x) * 4;
    if (idx + 4 > g_fb_len) {
        return;
    }
    g_fb[idx + 0] = (uint8_t)(color & 0xffu);
    g_fb[idx + 1] = (uint8_t)((color >> 8) & 0xffu);
    g_fb[idx + 2] = (uint8_t)((color >> 16) & 0xffu);
    g_fb[idx + 3] = (uint8_t)((color >> 24) & 0xffu);
}

static void clear_fb(uint32_t color) {
    if (g_fb == 0) {
        return;
    }
    for (size_t y = 0; y < g_h; y++) {
        for (size_t x = 0; x < g_w; x++) {
            put_px(x, y, color);
        }
    }
}

void tg_doom_bind_framebuffer(uint8_t *fb, size_t fb_len, size_t width, size_t height) {
    g_fb = fb;
    g_fb_len = fb_len;
    g_w = width;
    g_h = height;
    g_x = (int32_t)(width / 2);
    g_y = (int32_t)(height / 2);
}

int32_t tg_doom_step(uint32_t ticks_ms, int32_t key) {
    if (g_fb == 0 || g_w == 0 || g_h == 0) {
        return -1;
    }

    if (key == 'q' || key == 'Q') {
        return 1;
    }
    if (key == 'a' || key == 'h') {
        g_x -= 4;
    }
    if (key == 'd' || key == 'l') {
        g_x += 4;
    }
    if (key == 'w' || key == 'k') {
        g_y -= 4;
    }
    if (key == 's' || key == 'j') {
        g_y += 4;
    }

    if (g_x < 0) {
        g_x = 0;
    }
    if (g_y < 0) {
        g_y = 0;
    }
    if ((size_t)g_x >= g_w) {
        g_x = (int32_t)(g_w - 1);
    }
    if ((size_t)g_y >= g_h) {
        g_y = (int32_t)(g_h - 1);
    }

    clear_fb(0xff101418u);

    size_t bar_h = g_h / 12;
    if (bar_h < 8) {
        bar_h = 8;
    }
    size_t span = g_h / 3 + 1;
    size_t y0 = g_h / 3 + (ticks_ms / 16u) % span;
    if (y0 + bar_h > g_h) {
        y0 = g_h - bar_h;
    }
    for (size_t y = y0; y < y0 + bar_h && y < g_h; y++) {
        for (size_t x = 0; x < g_w; x++) {
            put_px(x, y, 0xffd7263du);
        }
    }

    size_t dot = g_h / 40;
    if (dot < 4) {
        dot = 4;
    }
    for (size_t y = (size_t)g_y; y < (size_t)g_y + dot && y < g_h; y++) {
        for (size_t x = (size_t)g_x; x < (size_t)g_x + dot && x < g_w; x++) {
            put_px(x, y, 0xfff3f3f3u);
        }
    }

    return 0;
}

int32_t tg_doom_full_available(void) {
#ifdef TG_DOOM_FULL
    return 1;
#else
    return 0;
#endif
}

int32_t tg_doom_full_init(void) {
#ifdef TG_DOOM_FULL
    static char arg0[] = "doom";
    static char arg1[] = "-iwad";
    static char arg2[] = "doom1.wad";
    char *argv[] = {arg0, arg1, arg2};
    doomgeneric_Create(3, argv);

    key_up = 'w';
    key_down = 's';
    key_left = 'a';
    key_right = 'd';
    key_strafeleft = 'z';
    key_straferight = 'x';
    key_fire = 'j';
    key_use = ' ';

    key_menu_up = 'w';
    key_menu_down = 's';
    key_menu_left = 'a';
    key_menu_right = 'd';
    key_menu_forward = KEY_ENTER;
    key_menu_confirm = 'y';  // Doom says press y to quit
    key_menu_abort = 'n';
    key_menu_back = KEY_ESCAPE;
    key_menu_activate = KEY_ESCAPE;

    return 0;
#else
    return -1;
#endif
}

int32_t tg_doom_full_tick(void) {
#ifdef TG_DOOM_FULL
    doomgeneric_Tick();
    return 0;
#else
    return -1;
#endif
}

void DG_Init(void) {
    TgFramebufferInfo info;
    info.ptr = 0;
    info.len = 0;
    info.width = 0;
    info.height = 0;
    tg_set_input_mode_polling();
    if (tg_framebuffer_info(&info) == 0 && info.ptr != 0 && info.len > 0 && info.width > 0 && info.height > 0) {
        tg_doom_bind_framebuffer(info.ptr, (size_t)info.len, (size_t)info.width, (size_t)info.height);
    }
}

void DG_DrawFrame(void) {
    if (g_fb == 0 || DG_ScreenBuffer == 0) {
        return;
    }

    size_t copy_w = DOOMGENERIC_RESX;
    size_t copy_h = DOOMGENERIC_RESY;
    if (copy_w > g_w) {
        copy_w = g_w;
    }
    if (copy_h > g_h) {
        copy_h = g_h;
    }

    size_t dst_pitch = g_w * 4;
    size_t src_pitch = DOOMGENERIC_RESX * 4;
    for (size_t y = 0; y < copy_h; y++) {
        uint8_t *dst = g_fb + y * dst_pitch;
        uint8_t *src = ((uint8_t *)DG_ScreenBuffer) + y * src_pitch;
        for (size_t x = 0; x < copy_w * 4; x++) {
            dst[x] = src[x];
        }
    }

    tg_framebuffer_flush();
}

void DG_SleepMs(uint32_t ms) {
    tg_sleep_ms(ms);
}

uint32_t DG_GetTicksMs(void) {
    return tg_get_ticks_ms();
}

int DG_GetKey(int *pressed, unsigned char *key) {
    poll_keys_once();
    if (g_key_read == g_key_write) {
        return 0;
    }
    unsigned short key_data = g_key_queue[g_key_read];
    g_key_read = (g_key_read + 1u) % 64u;
    *pressed = (int)(key_data >> 8);
    *key = (unsigned char)(key_data & 0xffu);
    return 1;
}

void DG_SetWindowTitle(const char *title) {
    (void)title;
}
