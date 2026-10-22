/* dyld section discovery only: this probe never unwinds or throws an exception. */
typedef unsigned long uintptr_t;
typedef struct {
    const void *header, *dwarf;
    uintptr_t dwarf_size;
    const void *compact;
    uintptr_t compact_size;
} UnwindSections;
extern _Bool _dyld_find_unwind_sections(const void *, UnwindSections *);
extern const void *_dyld_get_image_header(unsigned),
    *dyld_image_header_containing_address(const void *);
extern const char *_dyld_get_image_name(unsigned),
    *dyld_image_path_containing_address(const void *);
extern unsigned _dyld_image_count(void);
extern int printf(const char *, ...), puts(const char *), strcmp(const char *, const char *);
extern void *dlopen(const char *, int), *dlsym(void *, const char *);
extern int dlclose(void *);

volatile long unwind_data = 7;
__attribute__((noinline)) long unwind_leaf(long value) {
#ifdef UNWIND_FORCE_DWARF
    /* DW_CFA_def_cfa_expression: DW_OP_breg6(RBP) +16. This preserves the
     * frame-pointer CFA but requires a DWARF FDE rather than compact encoding. */
    __asm__ volatile(".cfi_escape 0x0f, 0x02, 0x76, 0x10");
#endif
    volatile long frame[8];
    for (int i = 0; i < 8; i++)
        frame[i] = value + i;
    return frame[(unsigned long)value % 8];
}

#ifdef UNWIND_PLUGIN
static long ready;
__attribute__((constructor)) static void plugin_constructor(void) {
    UnwindSections info;
    ready = _dyld_find_unwind_sections((const void *)unwind_leaf, &info) &&
            info.header == dyld_image_header_containing_address((const void *)unwind_leaf) &&
            (info.dwarf_size || info.compact_size);
}
long unwind_plugin_ready(void) { return ready; }
#else
#ifndef DIFF_CASE_COUNT
#define DIFF_CASE_COUNT 8
__attribute__((constructor)) static void constructor(void) { puts("unwind:constructor"); }
#endif
static const char *families[8] = {"function", "data",    "null", "header",
                                  "dwarf",    "compact", "path", "foreign"};
int main(int argc, char **argv) {
    void *plugin = 0;
    const void *function = (const void *)unwind_leaf, *data = (const void *)&unwind_data;
    unsigned image = 0;
    if (argc == 2) {
        plugin = dlopen(argv[1], 2);
        if (!plugin)
            return 31;
        function = dlsym(plugin, "unwind_leaf");
        data = dlsym(plugin, "unwind_data");
        long (*ready)(void) = (long (*)(void))dlsym(plugin, "unwind_plugin_ready");
        if (!function || !data || !ready || ready() != 1)
            return 32;
        image = _dyld_image_count() - 1;
    }
    const void *header = _dyld_get_image_header(image);
    for (int index = 0; index < DIFF_CASE_COUNT; index++) {
        long out[4] = {0};
        struct {
            uintptr_t before;
            UnwindSections info;
            uintptr_t after;
        } guarded = {0x12345678, {0}, 0x23456789};
        UnwindSections *info = &guarded.info;
        const void *address = index % 8 == 1 ? data : index % 8 == 3 ? header : function;
        if (index % 8 == 2 || index % 8 == 7) {
            info->header = (const void *)1;
            info->dwarf = (const void *)2;
            info->dwarf_size = 3;
            info->compact = (const void *)4;
            info->compact_size = 5;
            const void *absent = index % 8 == 2 ? (const void *)0 : (const void *)&guarded;
            out[0] = _dyld_find_unwind_sections(absent, info);
            out[1] = info->header == (const void *)1 && info->dwarf == (const void *)2 &&
                     info->dwarf_size == 3 && info->compact == (const void *)4 &&
                     info->compact_size == 5;
            out[2] = dyld_image_header_containing_address(absent) == (const void *)0;
            out[3] = dyld_image_path_containing_address(absent) == (const void *)0;
        } else {
            _Bool found = _dyld_find_unwind_sections(address, info);
            out[0] = found;
            out[1] = found && info->header == header &&
                     info->header == dyld_image_header_containing_address(address);
            out[2] = found && guarded.before == 0x12345678 && guarded.after == 0x23456789;
            out[3] = sizeof(UnwindSections) == 40;
            if (index % 8 == 4) {
                out[1] = info->dwarf != (const void *)0;
                out[2] = (long)info->dwarf_size;
                out[3] = info->dwarf_size >= 4 ? *(const unsigned *)info->dwarf : 0;
            } else if (index % 8 == 5) {
                out[1] = info->compact != (const void *)0;
                out[2] = (long)info->compact_size;
                out[3] = info->compact_size >= 4 ? *(const unsigned *)info->compact : 0;
            } else if (index % 8 == 6) {
                const char *name = dyld_image_path_containing_address(address);
                out[1] = name && !strcmp(name, _dyld_get_image_name(image));
            }
        }
        printf("{\"id\":\"%s:%d\",\"family\":\"%s\",\"index\":%d,\"result\":[%ld,%ld,%ld,%ld]}\n",
               families[index % 8], index, families[index % 8], index, out[0], out[1], out[2],
               out[3]);
    }
    if (plugin && dlclose(plugin))
        return 33;
    return 0;
}
#endif
