extern int puts(const char *);
extern int printf(const char *, ...);
extern void *dlopen(const char *, int);
extern void *dlsym(void *, const char *);
extern int dlclose(void *);
extern char *dlerror(void);
extern unsigned int _dyld_image_count(void);

static int contains(const char *text, const char *needle) {
    for (; *text; text++) {
        const char *a = text;
        const char *b = needle;
        while (*a && *b && *a == *b) {
            a++;
            b++;
        }
        if (*b == '\0') {
            return 1;
        }
    }
    return 0;
}

#define RTLD_LAZY 0x1
#define RTLD_NOW 0x2
#define RTLD_LOCAL 0x4
#define RTLD_NOLOAD 0x10
#define RTLD_FIRST 0x100
#define RTLD_DEFAULT ((void *)-2)
#define RTLD_MAIN_ONLY ((void *)-5)

int host_marker(void) { return 5; }

int main(int argc, char **argv) {
    if (argc != 2) {
        return 90;
    }
    unsigned int images = _dyld_image_count();
    void *handle = dlopen(argv[1], RTLD_NOW);
    if (handle == (void *)0) {
        printf("open: %s\n", dlerror());
        return 91;
    }
    if (_dyld_image_count() != images + 1) {
        return 107;
    }
    int (*add)(int, int) = (int (*)(int, int))dlsym(handle, "plugin_add");
    int *data = (int *)dlsym(handle, "plugin_data");
    const char *(*name)(void) = (const char *(*)(void))dlsym(handle, "plugin_name");
    if (add == (void *)0 || data == (void *)0 || name == (void *)0) {
        printf("sym: %s\n", dlerror());
        return 92;
    }
    printf("plugin:%d:%d:%s\n", add(2, 3), *data, name());

    if (dlsym(RTLD_MAIN_ONLY, "host_marker") == (void *)0) {
        return 109;
    }
    if (dlsym(RTLD_MAIN_ONLY, "plugin_add") != (void *)0) {
        return 110;
    }
    if (dlsym(handle, "plugin_absent") != (void *)0) {
        return 111;
    }
    char *missing_symbol = dlerror();
    if (missing_symbol == (void *)0 || !contains(missing_symbol, "not found")) {
        return 112;
    }

    void *again = dlopen(argv[1], RTLD_NOW | RTLD_NOLOAD);
    if (again != handle) {
        return 93;
    }
    if (dlclose(again) != 0 || dlclose(handle) != 0) {
        return 94;
    }
    if (dlsym(handle, "plugin_add") != (void *)0) {
        return 95;
    }
    if (dlerror() == (void *)0) {
        return 96;
    }

    if (dlopen("adapter-definitely-absent.dylib", RTLD_NOW) != (void *)0) {
        return 97;
    }
    char *missing = dlerror();
    if (missing == (void *)0 || !contains(missing, "not found")) {
        return 98;
    }
    if (dlerror() != (void *)0) {
        return 99;
    }

    if (dlsym(RTLD_DEFAULT, "host_marker") == (void *)0) {
        return 100;
    }
    if (dlsym(RTLD_DEFAULT, "malloc") == (void *)0) {
        return 101;
    }
    if (dlopen((void *)0, RTLD_NOW) != RTLD_DEFAULT) {
        return 102;
    }

    void *local = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL | RTLD_FIRST);
    if (local == (void *)0) {
        return 103;
    }
    if (dlsym(local, "plugin_add") == (void *)0) {
        return 104;
    }
    if (dlsym(local, "malloc") != (void *)0) {
        return 113;
    }
    if (dlsym(RTLD_DEFAULT, "plugin_add") != (void *)0) {
        return 105;
    }
    if (dlclose(local) != 0) {
        return 106;
    }

    puts("dl:passed");
    return 0;
}
