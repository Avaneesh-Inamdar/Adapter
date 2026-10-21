/* SDK-free Darwin LP64 differential consumer. Build only; capture runs it.
 * Every case gets its own xorshift32 seed, independent of host state.
 * No libc result below is computed by the generator as an expected answer. */
typedef unsigned long size_t;
typedef long time_t;
struct tm {
    int tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year;
    int tm_wday, tm_yday, tm_isdst;
    long tm_gmtoff;
    char *tm_zone;
};
extern size_t strlen(const char *);
extern int strcmp(const char *, const char *);
extern int memcmp(const void *, const void *, size_t);
extern void *memmove(void *, const void *, size_t);
extern long strtol(const char *, char **, int);
extern int snprintf(char *, size_t, const char *, ...);
extern int printf(const char *, ...);
extern struct tm *gmtime_r(const time_t *, struct tm *);
extern time_t timegm(struct tm *);
extern int *__error(void);
_Static_assert(sizeof(long) == 8 && sizeof(void *) == 8, "Darwin LP64 required");
_Static_assert(sizeof(unsigned int) == 4, "xorshift32 requires uint32");
_Static_assert(sizeof(struct tm) == 56, "Darwin x86-64 tm ABI");
_Static_assert(__builtin_offsetof(struct tm, tm_gmtoff) == 40, "tm_gmtoff ABI");
_Static_assert(__builtin_offsetof(struct tm, tm_zone) == 48, "tm_zone ABI");
#ifndef DIFF_CASE_COUNT
#define DIFF_CASE_COUNT 4096
#endif
_Static_assert(DIFF_CASE_COUNT >= 8 && DIFF_CASE_COUNT % 8 == 0, "balanced cases");

static const char *const families[8] = {
    "strlen", "strcmp", "memcmp", "memmove", "strtol", "snprintf", "gmtime_r", "timegm"
};
static unsigned int next(unsigned int *state) {
    unsigned int x = *state;
    x ^= x << 13; x ^= x >> 17; x ^= x << 5;
    *state = x;
    return x;
}
static unsigned int bytehash(const unsigned char *p, size_t n) {
    unsigned int hash = 2166136261u;
    for (size_t i = 0; i < n; ++i) { hash ^= p[i]; hash *= 16777619u; }
    return hash;
}
static int signof(int value) { return (value > 0) - (value < 0); }
static void emit(unsigned int index, const long long *result, unsigned int count) {
    const char *family = families[index % 8u];
    printf("{\"id\":\"%s:%u\",\"family\":\"%s\",\"index\":%u,\"result\":[",
           family, index, family, index);
    for (unsigned int j = 0; j < count; ++j)
        printf(j ? ",%lld" : "%lld", result[j]);
    printf("]}\n");
}
static void tmfields(long long *out, const struct tm *t) {
    out[0] = t->tm_sec; out[1] = t->tm_min; out[2] = t->tm_hour;
    out[3] = t->tm_mday; out[4] = t->tm_mon; out[5] = t->tm_year;
    out[6] = t->tm_wday; out[7] = t->tm_yday; out[8] = t->tm_isdst;
    out[9] = t->tm_gmtoff;
    /* tm_zone addresses and abbreviations are intentionally not serialized. */
}

int main(void) {
    for (unsigned int i = 0; i < DIFF_CASE_COUNT; ++i) {
        unsigned int state = 0x6d2b79f5u ^ (i * 0x9e3779b9u);
        unsigned int ordinal = i / 8u;
        long long result[11] = {0};
        unsigned int result_count = 1;
        unsigned char a[256], b[256];
        switch (i % 8u) {
        case 0: {
            unsigned int n = next(&state) % 255u;
            for (unsigned int j = 0; j < n; ++j) a[j] = (unsigned char)(1u + next(&state) % 255u);
            a[n] = 0;
            result[0] = (long long)strlen((const char *)a);
            break;
        }
        case 1: {
            unsigned int n = next(&state) % 240u;
            for (unsigned int j = 0; j < n; ++j) a[j] = b[j] = (unsigned char)(1u + next(&state) % 255u);
            a[n] = b[n] = 0;
            if (ordinal % 4u == 1u && n) b[next(&state) % n] = 0;
            if (ordinal % 4u == 2u && n) a[next(&state) % n] = 0;
            if (ordinal % 4u == 3u && n) {
                unsigned int at = next(&state) % n;
                b[at] = (unsigned char)(1u + next(&state) % 255u);
            }
            result[0] = signof(strcmp((const char *)a, (const char *)b));
            break;
        }
        case 2: {
            unsigned int n = next(&state) % 257u;
            for (unsigned int j = 0; j < 256u; ++j) a[j] = b[j] = (unsigned char)next(&state);
            if (n && ordinal % 3u) {
                unsigned int at = next(&state) % n;
                b[at] ^= (unsigned char)(1u + next(&state) % 255u);
            }
            result[0] = signof(memcmp(a, b, n));
            break;
        }
        case 3: {
            unsigned char buffer[512];
            for (unsigned int j = 0; j < 512u; ++j) buffer[j] = (unsigned char)next(&state);
            unsigned int src = 96u + next(&state) % 96u;
            unsigned int n = next(&state) % 257u;
            int delta = (int)(next(&state) % 127u) - 63;
            unsigned int dst = (unsigned int)((int)src + delta);
            if (ordinal % 8u == 0u) dst = src;
            if (ordinal % 8u == 1u) n = 0;
            memmove(buffer + dst, buffer + src, n);
            result[0] = (long long)bytehash(buffer, sizeof(buffer));
            break;
        }
        case 4: {
            static const int bases[5] = {2, 8, 10, 16, 36};
            static const char digits[] = "0123456789abcdefghijklmnopqrstuvwxyz";
            char input[160], *end;
            unsigned int used = 0;
            int base = bases[ordinal % 5u];
            if (ordinal % 3u == 0u) { input[used++] = ' '; input[used++] = '\t'; }
            if (ordinal % 3u == 1u) input[used++] = '-';
            if (ordinal % 3u == 2u) input[used++] = '+';
            unsigned int n = 1u + next(&state) % 24u;
            if (ordinal % 7u == 0u) n = 100u; /* representable and overflow paths */
            if (ordinal % 7u == 1u) n = 0;    /* no conversion */
            if (base == 16 && ordinal % 7u == 2u) { input[used++] = '0'; input[used++] = 'x'; }
            for (unsigned int j = 0; j < n; ++j) input[used++] = digits[next(&state) % (unsigned int)base];
            input[used++] = '!'; input[used] = 0;
            *__error() = 0;
            long value = strtol(input, &end, base);
            result[0] = value; result[1] = (long long)(end - input); result[2] = *__error();
            result_count = 3;
            break;
        }
        case 5: {
            static const size_t capacities[8] = {0, 1, 2, 7, 16, 31, 64, 96};
            unsigned char buffer[96];
            for (unsigned int j = 0; j < 96u; ++j) buffer[j] = 0xa5u;
            size_t cap = capacities[ordinal % 8u];
            int width = (int)(next(&state) % 25u), precision = (int)(next(&state) % 9u);
            int integer = (int)(next(&state) % 2000001u) - 1000000;
            double real = (double)integer / 8.0; /* exact binary fractions */
            int written;
            switch (ordinal % 6u) {
            case 0: written = snprintf((char *)buffer, cap, "%*.*d", width, precision, integer); break;
            case 1: written = snprintf((char *)buffer, cap, "%+0*.*ld", width, precision, (long)integer * 1000003L); break;
            case 2: written = snprintf((char *)buffer, cap, "%#0*x", width, next(&state)); break;
            case 3: written = snprintf((char *)buffer, cap, "%*.*f", width, precision, real); break;
            case 4: written = snprintf((char *)buffer, cap, "%*.*g", width, precision, real); break;
            default: written = snprintf((char *)buffer, cap, "L:%-*.*s:%d", width, precision, "aBc_12345", integer); break;
            }
            result[0] = written; result[1] = (long long)bytehash(buffer, sizeof(buffer));
            result_count = 2;
            break;
        }
        case 6: {
            static const time_t boundaries[8] = {0, -1, 1, -2147483648L, 2147483647L, 951782400L, -2208988800L, 4102444800L};
            time_t epoch = boundaries[ordinal % 8u];
            if (ordinal >= 8u) epoch = (time_t)(next(&state) % 4000000000u) - 2000000000L;
            struct tm t = {0};
            result[0] = gmtime_r(&epoch, &t) != (void *)0;
            tmfields(result + 1, &t);
            result_count = 11;
            break;
        }
        default: {
            struct tm t = {0};
            t.tm_sec = (int)(next(&state) % 301u) - 120;
            t.tm_min = (int)(next(&state) % 301u) - 120;
            t.tm_hour = (int)(next(&state) % 121u) - 48;
            t.tm_mday = (int)(next(&state) % 94u) - 31;
            t.tm_mon = (int)(next(&state) % 60u) - 24;
            t.tm_year = (int)(next(&state) % 301u) - 100;
            t.tm_isdst = (int)(ordinal % 3u) - 1;
            t.tm_wday = 8; t.tm_yday = 367;
            result[0] = timegm(&t);
            tmfields(result + 1, &t);
            result_count = 11;
            break;
        }
        }
        emit(i, result, result_count);
    }
    return 0;
}
