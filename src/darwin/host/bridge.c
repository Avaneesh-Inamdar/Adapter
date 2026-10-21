/* ABI adapters, independently implemented from public Apple layouts. */
#include "bridge.h"
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <io.h>
#include <fcntl.h>
#include <limits.h>
#include <math.h>
#include <windows.h>
#include <bcrypt.h>

static DbFile standard[3];
DbFile *db_stdin = &standard[0], *db_stdout = &standard[1], *db_stderr = &standard[2];
static uintptr_t stack_guard;

/* Darwin LP64 RuneLocale: the cached classification array starts at 0x3c. */
typedef struct {
    char magic[8], encoding[32];
    void *get, *put;
    int invalid;
    uint32_t types[256];
    int lower[256], upper[256];
    struct {
        int count;
        void *ranges;
    } ext[3];
    void *variable;
    int variable_length, class_count;
    void *classes;
} DbRunes;
_Static_assert(offsetof(DbRunes, types) == 60, "Darwin rune table");
_Static_assert(sizeof(DbRunes) == 3208, "Darwin RuneLocale");
static DbRunes runes;

void db_host_error(void) {
    int e;
    switch (errno) {
    case 0:
        e = 5;
        break;
    case ENOENT:
        e = 2;
        break;
    case EACCES:
        e = 13;
        break;
    case EBADF:
        e = 9;
        break;
    case ENOMEM:
        e = 12;
        break;
    case EEXIST:
        e = 17;
        break;
    case EINVAL:
        e = 22;
        break;
    case ENOSPC:
        e = 28;
        break;
    case ERANGE:
        e = 34;
        break;
    case EPIPE:
        e = 32;
        break;
    case EMFILE:
        e = 24;
        break;
    case ENOTDIR:
        e = 20;
        break;
    case EISDIR:
        e = 21;
        break;
    case EXDEV:
        e = 18;
        break;
    case ENOTEMPTY:
        e = 66;
        break;
    case EPERM:
        e = 1;
        break;
    case EROFS:
        e = 30;
        break;
    default:
        e = 5;
        break;
    }
    db_set_errno(e);
}
void db_file_status(DbFile *f) {
    if (feof(f->host))
        f->flags |= 0x20;
    if (ferror(f->host)) {
        f->flags |= 0x40;
        db_host_error();
    }
}
static int valid(DbFile *f) {
    if (!f || !f->host) {
        db_set_errno(9);
        return 0;
    }
    return 1;
}
static void init_file(DbFile *f, FILE *host, int owned, int flags) {
    memset(f, 0, sizeof(*f));
    f->host = host;
    f->owned = owned;
    f->file = (short)_fileno(host);
    f->flags = (short)flags;
}
void db_libc_begin(void) {
    init_file(db_stdin, stdin, 0, 4);
    init_file(db_stdout, stdout, 0, 8);
    init_file(db_stderr, stderr, 0, 10);
    _setmode(0, _O_BINARY);
    _setmode(1, _O_BINARY);
    _setmode(2, _O_BINARY);
    memset(&runes, 0, sizeof(runes));
    memcpy(runes.magic, "RuneMagA", 8);
    memcpy(runes.encoding, "NONE", 5);
    runes.invalid = 0xfffd;
    for (int c = 0; c < 256; c++) {
        uint32_t v = 0;
        if (c < 32 || c == 127)
            v |= 0x200;
        if (c == 32 || (c >= 9 && c <= 13))
            v |= 0x4000;
        if (c == 9 || c == 32)
            v |= 0x20000;
        if (c >= 32 && c < 127)
            v |= 0x40000;
        if (c > 32 && c < 127)
            v |= 0x800;
        if ((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z'))
            v |= 0x100;
        if (c >= 'A' && c <= 'Z')
            v |= 0x8000;
        if (c >= 'a' && c <= 'z')
            v |= 0x1000;
        if (c >= '0' && c <= '9')
            v |= 0x10400 | (c - '0');
        if (c >= 'A' && c <= 'F')
            v |= 0x10000 | (c - 'A' + 10);
        if (c >= 'a' && c <= 'f')
            v |= 0x10000 | (c - 'a' + 10);
        if ((v & 0x800) && !(v & 0x500))
            v |= 0x2000;
        runes.types[c] = v;
        runes.lower[c] = (c >= 'A' && c <= 'Z') ? c + 32 : c;
        runes.upper[c] = (c >= 'a' && c <= 'z') ? c - 32 : c;
    }
    if (BCryptGenRandom(NULL, (unsigned char *)&stack_guard, sizeof(stack_guard),
                        BCRYPT_USE_SYSTEM_PREFERRED_RNG) < 0)
        stack_guard = 0xdba1382c4900e700ULL;
    db_system_begin();
}
void db_libc_flush(void) { fflush(NULL); }
static G char *db_setlocale(int category, const char *name) {
    static char c[] = "C";
    static const char *categories[] = {"LC_ALL",     "LC_COLLATE", "LC_CTYPE",   "LC_MONETARY",
                                       "LC_NUMERIC", "LC_TIME",    "LC_MESSAGES"};
    if (category < 0 || category > 6) {
        db_set_errno(22);
        return NULL;
    }
    if (!name)
        return c;
    if (!*name) {
        name = db_getenv("LC_ALL");
        if ((!name || !*name) && category == 0) {
            const char *lang = db_getenv("LANG");
            for (int i = 1; i <= 6; i++) {
                const char *part = db_getenv(categories[i]);
                if (!part || !*part)
                    part = lang;
                if (part && *part && strcmp(part, "C") && strcmp(part, "POSIX")) {
                    db_set_errno(2);
                    return NULL;
                }
            }
            return c;
        }
        if (!name || !*name)
            name = db_getenv(categories[category]);
        if (!name || !*name)
            name = db_getenv("LANG");
        if (!name || !*name)
            name = "C";
    }
    if (!strcmp(name, "C") || !strcmp(name, "POSIX"))
        return c;
    db_set_errno(2);
    return NULL;
}
static G int db_maskrune(int c, uint64_t mask) {
    return c >= 0 && c < 256 ? (int)(runes.types[c] & mask) : 0;
}

G int db_fputc(int c, DbFile *f) {
    if (!valid(f))
        return -1;
    int v = fputc(c, f->host);
    db_file_status(f);
    return v;
}
G size_t db_fwrite(const void *p, size_t s, size_t n, DbFile *f) {
    if (!valid(f))
        return 0;
    if (s && n > SIZE_MAX / s) {
        db_set_errno(22);
        return 0;
    }
    size_t v = fwrite(p, s, n, f->host);
    db_file_status(f);
    return v;
}
static G size_t db_fread(void *p, size_t s, size_t n, DbFile *f) {
    if (!valid(f))
        return 0;
    if (s && n > SIZE_MAX / s) {
        db_set_errno(22);
        return 0;
    }
    size_t v = fread(p, s, n, f->host);
    db_file_status(f);
    return v;
}
static G int db_fputs(const char *s, DbFile *f) {
    return db_fwrite(s, 1, strlen(s), f) == strlen(s) ? 0 : -1;
}
G int db_puts(const char *s) { return db_fputs(s, db_stdout) < 0 ? -1 : db_fputc('\n', db_stdout); }
static G int db_putchar(int c) { return db_fputc(c, db_stdout); }
static G int db_getc(DbFile *f) {
    if (!valid(f))
        return -1;
    f->r = 0;
    int v = fgetc(f->host);
    db_file_status(f);
    return v;
}
static G int db_swbuf(int c, DbFile *f) {
    f->w = 0;
    return db_fputc(c, f);
}
static G char *db_fgets(char *p, int n, DbFile *f) {
    if (!valid(f) || n <= 0)
        return NULL;
    char *v = fgets(p, n, f->host);
    db_file_status(f);
    return v;
}
static G int db_ferror(DbFile *f) { return valid(f) ? !!(f->flags & 0x40) : 1; }
static G int db_feof(DbFile *f) { return valid(f) ? !!(f->flags & 0x20) : 0; }
static G void db_clearerr(DbFile *f) {
    if (valid(f)) {
        clearerr(f->host);
        f->flags &= ~0x60;
    }
}
static G int db_fflush(DbFile *f) {
    if (f && !valid(f))
        return -1;
    int v = fflush(f ? f->host : NULL);
    if (v)
        db_host_error();
    return v;
}
static G int db_fileno(DbFile *f) { return valid(f) ? f->file : -1; }
static G int db_fclose(DbFile *f) {
    if (!valid(f))
        return -1;
    int v = fclose(f->host);
    f->host = NULL;
    f->flags = 0;
    if (v)
        db_host_error();
    if (f->owned)
        free(f);
    return v;
}
static int mode_flags(const char *mode) {
    if (!mode || !*mode || !strchr("rwa", *mode)) {
        db_set_errno(22);
        return 0;
    }
    for (const char *p = mode + 1; *p; p++)
        if (!strchr("b+x", *p)) {
            db_set_errno(22);
            return 0;
        }
    return strchr(mode, '+') ? 16 : (*mode == 'r' ? 4 : 8);
}
static wchar_t *wide(const char *s) {
    int n = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, s, -1, NULL, 0);
    if (!n) {
        db_set_errno(92);
        return NULL;
    }
    wchar_t *p = malloc((size_t)n * sizeof(wchar_t));
    if (!p) {
        db_set_errno(12);
        return NULL;
    }
    MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, s, -1, p, n);
    return p;
}
wchar_t *db_path(const char *s) {
    /* /C/... is the explicit guest spelling of Windows drive C. Keeping the
     * leading slash lets unchanged POSIX consumers recognize absolute paths. */
    if (s[0] == '/' && ((s[1] >= 'A' && s[1] <= 'Z') || (s[1] >= 'a' && s[1] <= 'z'))) {
        if (s[2] == '/' || s[2] == 0) {
            size_t n = strlen(s);
            char *mapped = malloc(n + 3);
            if (!mapped) {
                db_set_errno(12);
                return NULL;
            }
            mapped[0] = s[1];
            mapped[1] = ':';
            mapped[2] = '/';
            strcpy(mapped + 3, s[2] ? s + 3 : "");
            wchar_t *result = wide(mapped);
            free(mapped);
            return result;
        }
        if (s[2] == ':' && s[3] == '/')
            return wide(s + 1);
    }
    return wide(s);
}
static G DbFile *db_fopen(const char *path, const char *mode) {
    int flags = mode_flags(mode);
    if (!flags)
        return NULL;
    wchar_t *p = db_path(path), *m = wide(mode);
    if (!p || !m) {
        free(p);
        free(m);
        return NULL;
    }
    /* Binary mode even when guest omits 'b': Darwin streams never translate CRLF. */
    wchar_t binary[8];
    size_t len = wcslen(m);
    if (len > 5) {
        free(p);
        free(m);
        db_set_errno(22);
        return NULL;
    }
    wcscpy(binary, m);
    if (!wcschr(m, L'b'))
        wcscat(binary, L"b");
    FILE *host = _wfopen(p, binary);
    if (!host) {
        DWORD attributes = GetFileAttributesW(p);
        if (attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_DIRECTORY))
            db_set_errno(21);
        else
            db_host_error();
        free(p);
        free(m);
        return NULL;
    }
    free(p);
    free(m);
    DbFile *f = malloc(sizeof(*f));
    if (!f) {
        fclose(host);
        db_set_errno(12);
        return NULL;
    }
    init_file(f, host, 1, flags);
    return f;
}
static G DbFile *db_fdopen(int fd, const char *mode) {
    int flags = mode_flags(mode);
    if (!flags)
        return NULL;
    if (fd < 0 || _get_osfhandle(fd) == -1) {
        db_set_errno(9);
        return NULL;
    }
    FILE *host = _fdopen(fd, mode);
    if (!host) {
        db_host_error();
        return NULL;
    }
    _setmode(fd, _O_BINARY);
    DbFile *f = malloc(sizeof(*f));
    if (!f) {
        db_set_errno(12);
        fclose(host);
        return NULL;
    }
    init_file(f, host, 1, flags);
    return f;
}
static G char *db_strerror(int e) {
    switch (e) {
    case 0:
        return "Undefined error: 0";
    case 2:
        return "No such file or directory";
    case 5:
        return "Input/output error";
    case 9:
        return "Bad file descriptor";
    case 12:
        return "Cannot allocate memory";
    case 13:
        return "Permission denied";
    case 17:
        return "File exists";
    case 20:
        return "Not a directory";
    case 21:
        return "Is a directory";
    case 22:
        return "Invalid argument";
    case 24:
        return "Too many open files";
    case 28:
        return "No space left on device";
    case 32:
        return "Broken pipe";
    case 34:
        return "Result too large";
    case 45:
        return "Operation not supported";
    case 84:
        return "Value too large to be stored in data type";
    default:
        return "Unknown error";
    }
}
static G void db_perror(const char *s) {
    if (s && *s)
        fprintf(stderr, "%s: ", s);
    fprintf(stderr, "%s\n", db_strerror(db_get_errno()));
}
static G void db_abort(void) {
    fprintf(stderr, "adapter-run [abort]: guest aborted\n");
    ExitProcess(134);
}
static G void db_assert(const char *fn, const char *file, int line, const char *expr) {
    fprintf(stderr, "adapter-run [assert]: %s, %s:%d (%s)\n", expr, file, line, fn);
    ExitProcess(134);
}
G void db_stack_fail(void) {
    fprintf(stderr, "adapter-run [stack]: guest stack check failed\n");
    ExitProcess(134);
}
static G void *db_calloc(size_t n, size_t s) {
    void *v = calloc(n, s);
    if (!v && n && s)
        db_set_errno(12);
    return v;
}
static G void *db_realloc(void *p, size_t n) {
    void *v = realloc(p, n);
    if (!v && n)
        db_set_errno(12);
    return v;
}
static G void *db_memmove(void *d, const void *s, size_t n) { return memmove(d, s, n); }
static G void *db_memset(void *d, int c, size_t n) { return memset(d, c, n); }
static G void db_bzero(void *d, size_t n) { memset(d, 0, n); }
static G void *db_memchr(const void *s, int c, size_t n) { return memchr(s, c, n); }
static G void *db_memmem(const void *h, size_t hn, const void *n, size_t nn) {
    if (!nn)
        return (void *)h;
    if (nn > hn)
        return NULL;
    for (size_t i = 0; i <= hn - nn; i++)
        if (!memcmp((const char *)h + i, n, nn))
            return (char *)h + i;
    return NULL;
}
static G void db_pattern(void *d, const void *pattern, size_t n) {
    unsigned char copy[16];
    memcpy(copy, pattern, 16);
    for (size_t i = 0; i < n; i++)
        ((unsigned char *)d)[i] = copy[i % 16];
}
static G void *db_memcpy_chk(void *d, const void *s, size_t n, size_t size) {
    if (n > size)
        db_stack_fail();
    return memcpy(d, s, n);
}
static G int db_strcmp(const char *a, const char *b) { return strcmp(a, b); }
static G int db_strncmp(const char *a, const char *b, size_t n) { return strncmp(a, b, n); }
static G char *db_strchr(const char *s, int c) { return strchr(s, c); }
static G char *db_strrchr(const char *s, int c) { return strrchr(s, c); }
static G char *db_strcpy(char *d, const char *s) { return strcpy(d, s); }
static G char *db_strdup(const char *s) {
    char *p = _strdup(s);
    if (!p)
        db_set_errno(12);
    return p;
}
static G size_t db_strspn(const char *s, const char *a) { return strspn(s, a); }
static G int64_t db_strtol(const char *s, char **end, int base) {
    if (base != 0 && (base < 2 || base > 36)) {
        if (end)
            *end = (char *)s;
        db_set_errno(22);
        return 0;
    }
    errno = 0;
    char *parsed;
    int64_t v = _strtoi64(s, &parsed, base);
    if (end)
        *end = parsed;
    if (errno)
        db_host_error();
    else if (parsed == s)
        db_set_errno(22);
    return v;
}
static G int db_atoi(const char *s) { return (int)db_strtol(s, NULL, 10); }
static G uint32_t db_random(void) {
    uint32_t n;
    if (BCryptGenRandom(NULL, (unsigned char *)&n, 4, BCRYPT_USE_SYSTEM_PREFERRED_RNG) < 0)
        db_abort();
    return n;
}
typedef G int (*Compare)(const void *, const void *);
static void swap(unsigned char *a, unsigned char *b, size_t n) {
    for (size_t i = 0; i < n; i++) {
        unsigned char v = a[i];
        a[i] = b[i];
        b[i] = v;
    }
}
static void sift(unsigned char *b, size_t root, size_t n, size_t s, Compare cmp) {
    while (root < n / 2) {
        size_t child = root * 2 + 1;
        if (child + 1 < n && cmp(b + child * s, b + (child + 1) * s) < 0)
            child++;
        if (cmp(b + root * s, b + child * s) >= 0)
            return;
        swap(b + root * s, b + child * s, s);
        root = child;
    }
}
static G void db_qsort(void *b, size_t n, size_t s, Compare cmp) {
    if (n < 2 || !s)
        return;
    if (n > SIZE_MAX / s) {
        db_set_errno(22);
        return;
    }
    for (size_t i = n / 2; i > 0; i--)
        sift(b, i - 1, n, s, cmp);
    for (size_t end = n - 1; end > 0; end--) {
        swap(b, (unsigned char *)b + end * s, s);
        sift(b, 0, end, s, cmp);
    }
}

#define M1(n)                                                                                      \
    static G double db_##n(double x) { return n(x); }
#define M2(n)                                                                                      \
    static G double db_##n(double x, double y) { return n(x, y); }
M1(acos);
M1(acosh);
M1(asin);
M1(asinh);
M1(atan);
M1(atanh);
M1(cbrt);
M1(cos);
M1(cosh);
M1(erf);
M1(erfc);
M1(exp);
M1(exp2);
M1(expm1);
M1(lgamma);
M1(log);
M1(log10);
M1(log1p);
M1(log2);
M1(logb);
M1(sin);
M1(sinh);
M1(tan);
M1(tanh);
M1(tgamma);
M2(atan2);
M2(fdim);
M2(fmod);
M2(hypot);
M2(nextafter);
M2(pow);
M2(remainder);
static G double db_exp10(double x) { return pow(10, x); }
static G double db_fma(double x, double y, double z) { return fma(x, y, z); }
static G double db_frexp(double x, int *e) { return frexp(x, e); }
static G double db_modf(double x, double *i) { return modf(x, i); }
static G double db_ldexp(double x, int e) { return ldexp(x, e); }
static G double db_scalbln(double x, int64_t e) {
    return scalbn(x, e > INT_MAX ? INT_MAX : e < INT_MIN ? INT_MIN : (int)e);
}
static G double db_scalb(double x, double e) {
    if (isnan(e))
        return e;
    if (!isfinite(e))
        return e > 0 ? x * INFINITY : x * 0.0;
    if (trunc(e) != e)
        return NAN;
    return db_scalbln(x, e > INT_MAX ? INT_MAX : e < INT_MIN ? INT_MIN : (int64_t)e);
}
static G double db_j0(double x) { return _j0(x); }
static G double db_j1(double x) { return _j1(x); }
static G double db_y0(double x) { return _y0(x); }
static G double db_y1(double x) { return _y1(x); }
static G double db_jn(int n, double x) { return _jn(n, x); }
static G double db_yn(int n, double x) { return _yn(n, x); }
static G double db_lgamma_r(double x, int *sign) {
    *sign = x > 0 || sin(3.141592653589793 * x) > 0 ? 1 : -1;
    return lgamma(x);
}

uintptr_t db_libc_symbol(const char *name) {
    uintptr_t value = db_format_symbol(name);
    if (value)
        return value;
    value = db_system_symbol(name);
    if (value)
        return value;
    LOOKUP("__DefaultRuneLocale", &runes);
    LOOKUP("___stdinp", &db_stdin);
    LOOKUP("___stdoutp", &db_stdout);
    LOOKUP("___stderrp", &db_stderr);
    LOOKUP("___stack_chk_guard", &stack_guard);
    LOOKUP("___stack_chk_fail", db_stack_fail);
    LOOKUP("____chkstk_darwin", db_chkstk);
    LOOKUP("___assert_rtn", db_assert);
    LOOKUP("_abort", db_abort);
    LOOKUP("_exit", db_guest_exit);
    LOOKUP("_setlocale", db_setlocale);
    LOOKUP("___maskrune", db_maskrune);
    LOOKUP("_puts", db_puts);
    LOOKUP("_putchar", db_putchar);
    LOOKUP("_fputc", db_fputc);
    LOOKUP("_fputs", db_fputs);
    LOOKUP("_fwrite", db_fwrite);
    LOOKUP("_fread", db_fread);
    LOOKUP("_getc", db_getc);
    LOOKUP("___srget", db_getc);
    LOOKUP("___swbuf", db_swbuf);
    LOOKUP("_fgets", db_fgets);
    LOOKUP("_ferror", db_ferror);
    LOOKUP("_feof", db_feof);
    LOOKUP("_clearerr", db_clearerr);
    LOOKUP("_fflush", db_fflush);
    LOOKUP("_fileno", db_fileno);
    LOOKUP("_fclose", db_fclose);
    LOOKUP("_fopen$DARWIN_EXTSN", db_fopen);
    LOOKUP("_fopen", db_fopen);
    LOOKUP("_fdopen$DARWIN_EXTSN", db_fdopen);
    LOOKUP("_fdopen", db_fdopen);
    LOOKUP("_strerror", db_strerror);
    LOOKUP("_perror", db_perror);
    LOOKUP("_calloc", db_calloc);
    LOOKUP("_realloc", db_realloc);
    LOOKUP("_memmove", db_memmove);
    LOOKUP("_memset", db_memset);
    LOOKUP("___bzero", db_bzero);
    LOOKUP("_memchr", db_memchr);
    LOOKUP("_memmem", db_memmem);
    LOOKUP("_memset_pattern16", db_pattern);
    LOOKUP("___memcpy_chk", db_memcpy_chk);
    LOOKUP("_strcmp", db_strcmp);
    LOOKUP("_strncmp", db_strncmp);
    LOOKUP("_strchr", db_strchr);
    LOOKUP("_strrchr", db_strrchr);
    LOOKUP("_strcpy", db_strcpy);
    LOOKUP("_strdup", db_strdup);
    LOOKUP("_strspn", db_strspn);
    LOOKUP("_strtol", db_strtol);
    LOOKUP("_atoi", db_atoi);
    LOOKUP("_arc4random", db_random);
    LOOKUP("_qsort", db_qsort);
#define LM(n) LOOKUP("_" #n, db_##n)
    LM(acos);
    LM(acosh);
    LM(asin);
    LM(asinh);
    LM(atan);
    LM(atanh);
    LM(cbrt);
    LM(cos);
    LM(cosh);
    LM(erf);
    LM(erfc);
    LM(exp);
    LM(exp2);
    LM(expm1);
    LM(lgamma);
    LM(log);
    LM(log10);
    LM(log1p);
    LM(log2);
    LM(logb);
    LM(sin);
    LM(sinh);
    LM(tan);
    LM(tanh);
    LM(tgamma);
    LM(atan2);
    LM(fdim);
    LM(fmod);
    LM(hypot);
    LM(nextafter);
    LM(pow);
    LM(remainder);
    LM(fma);
    LM(frexp);
    LM(modf);
    LM(ldexp);
    LM(scalbln);
    LM(scalb);
    LM(j0);
    LM(j1);
    LM(jn);
    LM(y0);
    LM(y1);
    LM(yn);
    LM(lgamma_r);
    LOOKUP("___exp10", db_exp10);
    return 0;
}
