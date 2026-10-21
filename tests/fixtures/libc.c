/* Compiled for Darwin x86_64 without SDK headers: ABI assertions + consumers. */
typedef unsigned long size_t;
typedef struct {
    unsigned char *p;
    int r, w;
    short flags, file;
    char rest[132];
} File;
typedef struct {
    char magic[8], encoding[32];
    void *get, *put;
    int invalid;
    unsigned types[256];
    int lower[256], upper[256];
} Runes;
extern Runes _DefaultRuneLocale;
extern File *__stdinp, *__stdoutp, *__stderrp;
extern char **environ;
extern char *setlocale(int, const char *), *getenv(const char *), *strerror(int);
extern int setenv(const char *, const char *, int), unsetenv(const char *), *__error(void);
extern int __maskrune(int, unsigned long);
extern int puts(const char *), printf(const char *, ...), fprintf(File *, const char *, ...);
extern int snprintf(char *, size_t, const char *, ...),
    vsnprintf(char *, size_t, const char *, __builtin_va_list);
extern int sscanf(const char *, const char *, ...);
extern int strcmp(const char *, const char *), memcmp(const void *, const void *, size_t);
extern size_t strlen(const char *), fread(void *, size_t, size_t, File *),
    fwrite(const void *, size_t, size_t, File *);
extern int getc(File *), fputc(int, File *), fputs(const char *, File *), feof(File *),
    ferror(File *), fflush(File *), fileno(File *), fclose(File *);
extern void clearerr(File *);
extern int __srget(File *), __swbuf(int, File *);
extern double nexttoward(double, long double);
extern int __cxa_atexit(void (*)(void *), void *, void *);
extern void exit(int);
#define CHECK(expr)                                                                                \
    do {                                                                                           \
        if (!(expr)) {                                                                             \
            fprintf(__stderrp, "libc:check %d failed: %s\n", __LINE__, #expr);                     \
            return __LINE__ % 100 + 1;                                                             \
        }                                                                                          \
    } while (0)
static int format(char *out, size_t n, const char *fmt, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, fmt);
    int v = vsnprintf(out, n, fmt, a);
    __builtin_va_end(a);
    return v;
}
static void at_exit(void *value) {
    if (value == (void *)123)
        puts("libc:exit callback");
}
int main(int argc, char **argv) {
    CHECK(sizeof(File) == 152 && __stdinp->file == 0 && __stdoutp->file == 1 &&
          __stderrp->file == 2);
    if (argc > 1 && !strcmp(argv[1], "exit")) {
        CHECK(__cxa_atexit(at_exit, (void *)123, 0) == 0);
        exit(23);
    }
    CHECK(!memcmp(_DefaultRuneLocale.magic, "RuneMagA", 8));
    CHECK(_DefaultRuneLocale.types['A'] == (0x100 | 0x8000 | 0x10000 | 0x40000 | 0x800 | 10));
    CHECK(_DefaultRuneLocale.types['9'] == (0x400 | 0x10000 | 0x40000 | 0x800 | 9));
    CHECK(_DefaultRuneLocale.types[200] == 0 && _DefaultRuneLocale.lower['A'] == 'a' &&
          _DefaultRuneLocale.upper['z'] == 'Z');
    CHECK(__maskrune(' ', 0x4000) == 0x4000 && __maskrune(-1, ~0UL) == 0 &&
          __maskrune(256, ~0UL) == 0);
    CHECK(!strcmp(setlocale(0, "C"), "C") && !strcmp(setlocale(2, "POSIX"), "C") &&
          !strcmp(setlocale(4, 0), "C"));
    CHECK(setlocale(7, "C") == 0 && *__error() == 22);
    CHECK(setlocale(0, "not-a-locale") == 0 && !strcmp(setlocale(0, 0), "C"));
    CHECK(setenv("LC_ALL", "POSIX", 1) == 0 && !strcmp(setlocale(0, ""), "C"));
    CHECK(unsetenv("LC_ALL") == 0 && setenv("LANG", "C", 1) == 0);
    const char *categories[] = {"LC_COLLATE", "LC_CTYPE", "LC_MONETARY",
                                "LC_NUMERIC", "LC_TIME",  "LC_MESSAGES"};
    for (int i = 0; i < 6; i++)
        CHECK(setenv(categories[i], "C", 1) == 0);
    CHECK(setenv("LC_NUMERIC", "unsupported", 1) == 0 && setlocale(0, "") == 0);
    CHECK(setenv("LC_NUMERIC", "POSIX", 1) == 0 && !strcmp(setlocale(0, ""), "C"));
    CHECK(setenv("DB_EMPTY", "", 1) == 0 && getenv("DB_EMPTY") && !*getenv("DB_EMPTY"));
    CHECK(setenv("DB_EMPTY", "hello", 0) == 0 && !*getenv("DB_EMPTY"));
    CHECK(setenv("DB_EMPTY", "world", 1) == 0 && !strcmp(getenv("DB_EMPTY"), "world"));
    CHECK(unsetenv("DB_EMPTY") == 0 && !getenv("DB_EMPTY"));
    CHECK(setenv("bad=name", "x", 1) == -1 && *__error() == 22 && environ);
    char out[1024];
    int n = snprintf(out, sizeof(out), "%s:%d:%ld:%lld:%zu:%.3f:%c:%#x", "s", -7, 4294967297L,
                     -9000000000LL, (size_t)17, 1.25, 'Z', 255);
    CHECK(n == (int)strlen(out) && !strcmp(out, "s:-7:4294967297:-9000000000:17:1.250:Z:0xff"));
    n = snprintf(out, sizeof(out),
                 "%d,%d,%d,%d,%d,%d,%d,%d,%d,%d/%.1f,%.1f,%.1f,%.1f,%.1f,%.1f,%.1f,%.1f,%.1f,%.1f",
                 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0);
    CHECK(n > 0 && !strcmp(out, "1,2,3,4,5,6,7,8,9,10/1.0,2.0,3.0,4.0,5.0,6.0,7.0,8.0,9.0,10.0"));
    CHECK(format(out, sizeof(out), "%*.*f:%.*s:%hhd:%hd", -8, 2, 3.125, 3, "abcdef", 255, 65535) >
              0 &&
          !strcmp(out, "3.12    :abc:-1:-1"));
    CHECK(snprintf(out, 4, "abcdef") == 6 && !strcmp(out, "abc"));
    CHECK(snprintf(out, 1, "abcdef") == 6 && !out[0]);
    CHECK(snprintf(0, 0, "abc%d", 123) == 6);
    int count = -1;
    long wide = -1;
    signed char small = -1;
    CHECK(snprintf(out, sizeof(out), "abc%nDE%lnF%hhn", &count, &wide, &small) == 6 && count == 3 &&
          wide == 5 && small == 6);
    CHECK(snprintf(out, sizeof(out), "%2$d", 3, 4) == -1 && *__error() == 45);
    CHECK(snprintf(out, sizeof(out), "%Lf", 1.0L) == -1 && *__error() == 45);
    CHECK(nexttoward(1.0, 1.0L + 0x1p-63L) == 0x1.0000000000001p0);
    CHECK(nexttoward(1.0, 1.0L - 0x1p-63L) == 0x1.fffffffffffffp-1);
    CHECK(nexttoward(2.0, 2.0L) == 2.0);
    unsigned char input[6];
    CHECK(fread(input, 2, 3, __stdinp) == 3 && !memcmp(input, "A\r\n\0\xffZ", 6));
    CHECK(getc(__stdinp) == -1 && feof(__stdinp) && !ferror(__stdinp));
    CHECK((__stdinp->flags & 32) != 0);
    clearerr(__stdinp);
    CHECK(!feof(__stdinp) && !(__stdinp->flags & 32));
    CHECK(fileno(__stdoutp) == 1 && fflush(0) == 0);
    CHECK(fprintf(__stderrp, "libc:stderr %ld %.2f\n", 4294967297L, 1.25) > 0);
    CHECK(printf("libc:printf %d %.1f\n", 7, 2.5) > 0);
    CHECK(fputs("libc:slow", __stdoutp) >= 0);
    __stdoutp->w--;
    CHECK(__swbuf('\n', __stdoutp) == '\n');
    int parsed_i = 0;
    char parsed_s[16] = {0};
    double parsed_d = 0.0;
    int scanned = sscanf("99 apple 4.5", "%d %15s %lf", &parsed_i, parsed_s, &parsed_d);
    CHECK(scanned == 3 && parsed_i == 99 && !strcmp(parsed_s, "apple") && parsed_d == 4.5);
    CHECK(puts("libc:locale, stdio, varargs and long double passed") >= 0);
    return 0;
}
