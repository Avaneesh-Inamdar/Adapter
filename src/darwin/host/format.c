#include "bridge.h"
#include <stdlib.h>
#include <string.h>
#include <limits.h>
#include <errno.h>

static uint64_t integer(DbVa *a) {
    uint64_t v;
    if (a->gp <= 40) {
        memcpy(&v, a->registers + a->gp, 8);
        a->gp += 8;
    } else {
        memcpy(&v, a->overflow, 8);
        a->overflow += 8;
    }
    return v;
}
static double real(DbVa *a) {
    double v;
    if (a->fp <= 160) {
        memcpy(&v, a->registers + a->fp, 8);
        a->fp += 16;
    } else {
        memcpy(&v, a->overflow, 8);
        a->overflow += 8;
    }
    return v;
}
typedef struct {
    char *out;
    size_t size, total;
    DbFile *file;
    int error;
} Sink;
static void emit(Sink *s, const char *p, size_t n) {
    if (s->error)
        return;
    if (n > INT_MAX - s->total) {
        s->error = 1;
        db_set_errno(84);
        return;
    }
    if (s->file) {
        if (db_fwrite(p, 1, n, s->file) != n) {
            s->error = 1;
            return;
        }
    } else if (s->size && s->total < s->size - 1) {
        size_t copy = s->size - 1 - s->total;
        if (copy > n)
            copy = n;
        memcpy(s->out + s->total, p, copy);
    }
    s->total += n;
}
/* Parse one conversion then use a *host* typed vararg call for that conversion.
 * No Apple va_list or LP64 long is given to Microsoft's printf implementation.
 * Positional arguments, wide strings and 80-bit long double fail with ENOTSUP. */
static int format(Sink *sink, const char *fmt, DbVa args) {
    while (*fmt && !sink->error) {
        const char *start = fmt;
        while (*fmt && *fmt != '%')
            fmt++;
        emit(sink, start, (size_t)(fmt - start));
        if (!*fmt)
            break;
        fmt++;
        if (*fmt == '%') {
            emit(sink, fmt++, 1);
            continue;
        }
        char spec[96] = "%", length[3] = "";
        size_t pos = 1;
        while (*fmt && strchr("-+ #0", *fmt)) {
            if (pos > 12)
                goto unsupported;
            spec[pos++] = *fmt++;
        }
        if (*fmt == '*') {
            int width = (int)integer(&args);
            fmt++;
            if (width == INT_MIN)
                goto unsupported;
            if (width < 0) {
                spec[pos++] = '-';
                width = -width;
            }
            if (width > 1048576)
                goto unsupported;
            pos += (size_t)snprintf(spec + pos, sizeof(spec) - pos, "%d", width);
        } else {
            int width = 0;
            while (*fmt >= '0' && *fmt <= '9') {
                if (width > 104857)
                    goto unsupported;
                width = width * 10 + *fmt - '0';
                if (pos >= 40)
                    goto unsupported;
                spec[pos++] = *fmt++;
            }
            if (width > 1048576)
                goto unsupported;
        }
        if (*fmt == '.') {
            fmt++;
            int precision = 0;
            if (*fmt == '*') {
                precision = (int)integer(&args);
                fmt++;
            } else
                while (*fmt >= '0' && *fmt <= '9') {
                    if (precision > 104857)
                        goto unsupported;
                    precision = precision * 10 + *fmt++ - '0';
                }
            if (precision > 1048576)
                goto unsupported;
            if (precision >= 0)
                pos += (size_t)snprintf(spec + pos, sizeof(spec) - pos, ".%d", precision);
        }
        if (*fmt && strchr("hljztL", *fmt)) {
            length[0] = *fmt++;
            if ((*fmt == 'h' && length[0] == 'h') || (*fmt == 'l' && length[0] == 'l'))
                length[1] = *fmt++;
        }
        char conv = *fmt;
        if (!conv)
            goto unsupported;
        fmt++;
        int n = -1;
        char small[512], *chunk = small;
        size_t capacity = sizeof(small);
        uint64_t iv = 0;
        double dv = 0;
        void *pv = NULL;
        enum { NONE, INT, SINT, FLOAT, STR, PTR, CHAR } kind = NONE;
        if (strchr("diouxX", conv)) {
            iv = integer(&args);
            if (!length[0]) {
                iv = (conv == 'd' || conv == 'i') ? (uint64_t)(int64_t)(int32_t)iv : (uint32_t)iv;
            }
            if (length[0] == 'h') {
                if (length[1])
                    iv = (conv == 'd' || conv == 'i') ? (uint64_t)(int64_t)(int8_t)iv : (uint8_t)iv;
                else
                    iv = (conv == 'd' || conv == 'i') ? (uint64_t)(int64_t)(int16_t)iv
                                                      : (uint16_t)iv;
            }
            if (length[0] == 'L')
                goto unsupported;
            spec[pos++] = 'l';
            spec[pos++] = 'l';
            kind = (conv == 'd' || conv == 'i') ? SINT : INT;
        } else if (strchr("aAeEfFgG", conv)) {
            if (length[0] && length[0] != 'l')
                goto unsupported;
            dv = real(&args);
            kind = FLOAT;
        } else if (conv == 's' || conv == 'p' || conv == 'c') {
            if (length[0])
                goto unsupported;
            iv = integer(&args);
            pv = (void *)(uintptr_t)iv;
            kind = conv == 's' ? STR : conv == 'p' ? PTR : CHAR;
            if (kind == STR && !pv)
                pv = "(null)";
        } else if (conv == 'n') {
            pv = (void *)(uintptr_t)integer(&args);
            if (length[0] == 'h') {
                if (length[1])
                    *(signed char *)pv = (signed char)sink->total;
                else
                    *(short *)pv = (short)sink->total;
            } else if (!length[0])
                *(int *)pv = (int)sink->total;
            else if (strchr("ljzt", length[0]))
                *(int64_t *)pv = (int64_t)sink->total;
            else
                goto unsupported;
            continue;
        } else
            goto unsupported;
        spec[pos++] = conv;
        spec[pos] = 0;
        for (;;) {
            switch (kind) {
            case INT:
                n = snprintf(chunk, capacity, spec, (unsigned long long)iv);
                break;
            case SINT:
                n = snprintf(chunk, capacity, spec, (long long)iv);
                break;
            case FLOAT:
                n = snprintf(chunk, capacity, spec, dv);
                break;
            case STR:
                n = snprintf(chunk, capacity, spec, (const char *)pv);
                break;
            case PTR:
                n = snprintf(chunk, capacity, spec, pv);
                break;
            case CHAR:
                n = snprintf(chunk, capacity, spec, (int)iv);
                break;
            default:
                break;
            }
            if (n < 0) {
                sink->error = 1;
                db_set_errno(22);
                break;
            }
            if ((size_t)n < capacity) {
                emit(sink, chunk, (size_t)n);
                break;
            }
            if (chunk != small) {
                sink->error = 1;
                db_set_errno(22);
                break;
            }
            capacity = (size_t)n + 1;
            chunk = malloc(capacity);
            if (!chunk) {
                sink->error = 1;
                db_set_errno(12);
                break;
            }
        }
        if (chunk != small)
            free(chunk);
    }
    if (!sink->file && sink->size)
        sink->out[sink->total < sink->size ? sink->total : sink->size - 1] = 0;
    return sink->error ? -1 : (int)sink->total;
unsupported:
    db_set_errno(45);
    sink->error = 1;
    if (!sink->file && sink->size)
        sink->out[sink->total < sink->size ? sink->total : sink->size - 1] = 0;
    return -1;
}
int db_printf_entry(const char *fmt, DbVa *a) {
    Sink sink = {.file = db_stdout};
    return format(&sink, fmt, *a);
}
int db_fprintf_entry(DbFile *f, const char *fmt, DbVa *a) {
    if (!f || !f->host) {
        db_set_errno(9);
        return -1;
    }
    Sink sink = {.file = f};
    return format(&sink, fmt, *a);
}
int db_snprintf_entry(char *out, size_t size, const char *fmt, DbVa *a) {
    Sink sink = {.out = out, .size = size};
    return format(&sink, fmt, *a);
}
/* Fortified print requests the writable size as a separate check parameter:
 * __sprintf_chk aborts instead of overflowing; __snprintf_chk only forbids a
 * length argument that exceeds the real buffer (bounded writes stay safe). */
int db_sprintf_chk_entry(char *dst, size_t size, const char *fmt, DbVa *a) {
    if (!size)
        db_stack_fail();
    Sink sink = {.out = dst, .size = size};
    int v = format(&sink, fmt, *a);
    if (v >= 0 && sink.total >= size)
        db_stack_fail(); /* Apple __sprintf_chk aborts rather than truncating. */
    return v;
}
int db_snprintf_chk_entry(char *dst, size_t maxlen, size_t dst_size, const char *fmt, DbVa *a) {
    if (maxlen > dst_size)
        db_stack_fail(); /* Fortify misuse: larger write than the real buffer. */
    return db_snprintf_entry(dst, maxlen, fmt, a);
}

/* POSIX scanf over a SysV va_list. Unsupported conversions fail with ENOTSUP
 * before advancing arguments, so the guest sees a consistent state. This
 * implementation assigns only complete items, counts suppressed-but-complete
 * conversions toward the EOF rule, and rewinds non-consumed prefix characters,
 * as C11 7.21.6.2 requires. Scratch tokens are 512 bytes, so numeric tokens
 * are width-capped at 511 characters. */
static int is_space(int c) {
    return c == ' ' || c == '\t' || c == '\n' || c == '\v' || c == '\f' || c == '\r';
}
static int hex_value(int c) {
    if (c >= '0' && c <= '9')
        return c - '0';
    if (c >= 'a' && c <= 'f')
        return c - 'a' + 10;
    if (c >= 'A' && c <= 'F')
        return c - 'A' + 10;
    return -1;
}
static const char *skip_spaces(const char *s) {
    while (is_space((unsigned char)*s))
        s++;
    return s;
}
static int is_digit(int c) { return c >= '0' && c <= '9'; }
static void store_int(DbVa *a, uint64_t value, const char *length, int signed_conv) {
    void *target = (void *)(uintptr_t)integer(a);
    if (!target) {
        db_set_errno(22);
        return;
    }
    if (!length[0])
        *(int *)target = signed_conv ? (int)(int32_t)value : (int)(uint32_t)value;
    else if (length[0] == 'h' && length[1])
        *(signed char *)target = (signed char)value;
    else if (length[0] == 'h')
        *(short *)target = (short)value;
    else if (length[0] == 'l' || length[0] == 'z' || length[0] == 't' || length[0] == 'j')
        *(int64_t *)target = (int64_t)value;
    else { /* 'L' never reaches the store; the caller rejected it. */
    }
}
/* Build a numeric token: sign, optional radix prefix, then digits valid for the
 * resolved base. The cursor is left after the consumed prefix; NULL means the
 * input did not hold a complete number (matching failure, cursor on the start). */
static const char *collect_int(const char *in, char *token, size_t capacity, int width, int base,
                               int *resolved) {
    const char *s = skip_spaces(in);
    size_t max = capacity - 1;
    if (width && (size_t)width < max)
        max = (size_t)width;
    size_t n = 0;
    if (*s == '+' || *s == '-') {
        if (!max)
            return NULL;
        token[n++] = *s++;
    }
    *resolved = base;
    if (n < max && *s == '0' && (base == 0 || base == 16)) {
        token[n++] = *s++;
        if (n < max && (*s == 'x' || *s == 'X') && hex_value(s[1]) >= 0) {
            token[n++] = *s++;
            *resolved = 16;
        } else if (base == 0)
            *resolved = 8; /* 012 is octal under auto-detection. */
    }
    while (n < max && *s) {
        int value = hex_value(*s);
        if (value < 0 || value >= *resolved)
            break;
        token[n++] = *s++;
    }
    if (n <= (size_t)(token[0] == '+' || token[0] == '-'))
        return NULL; /* Sign with no digits, or nothing at all. */
    token[n] = 0;
    return s;
}
int db_sscanf_entry(const char *start, const char *fmt, DbVa *args) {
    if (!start || !fmt) {
        db_set_errno(22);
        return -1;
    }
    const char *p = start;
    int assigned = 0, completed = 0;
    while (*fmt) {
        if (is_space((unsigned char)*fmt)) {
            while (is_space((unsigned char)*fmt))
                fmt++;
            p = skip_spaces(p);
            continue;
        }
        if (*fmt != '%') {
            if (*p != *fmt) {
                return assigned; /* Matching failure stops the scan. */
            }
            p++;
            fmt++;
            continue;
        }
        fmt++;
        if (*fmt == '%') {
            if (*p != '%')
                return assigned;
            p++;
            fmt++;
            continue;
        }
        int suppress = 0, width = 0;
        if (*fmt == '*') {
            suppress = 1;
            fmt++;
        }
        while (is_digit(*fmt)) {
            if (width > 104857) {
                db_set_errno(45);
                return -1;
            }
            width = width * 10 + (*fmt++ - '0');
        }
        if (width > 511) {
            db_set_errno(45); /* Tokens are width-capped at the 512-byte scratch. */
            return -1;
        }
        if (*fmt == '$') {
            db_set_errno(45); /* Positional arguments are unsupported. */
            return -1;
        }
        char length[3] = "";
        while (*fmt && (strchr("hljzt", *fmt) || *fmt == 'L')) {
            if ((length[0] == 'h' && *fmt != 'h') || (length[0] == 'l' && *fmt != 'l')) {
                db_set_errno(45);
                return -1; /* Only hh and ll combine; the rest do not. */
            }
            if (length[0] == 'L' || (length[0] && length[1])) {
                db_set_errno(45); /* Extended forms beyond two characters. */
                return -1;
            }
            length[length[0] ? 1 : 0] = *fmt++;
        }
        if (*fmt == 'm') {
            db_set_errno(45); /* Assignment-allocation needs a guest allocator. */
            return -1;
        }
        char conv = *fmt;
        if (!conv)
            return assigned; /* A stray '%' at the end is treated as done. */
        fmt++;
        if (conv == 'n') {
            if (suppress)
                continue;
            void *target = (void *)(uintptr_t)integer(args);
            if (!target) {
                db_set_errno(22);
                return -1;
            }
            if (!length[0])
                *(int *)target = (int)(p - start);
            else if (length[0] == 'h' && length[1])
                *(signed char *)target = (signed char)(p - start);
            else if (length[0] == 'h')
                *(short *)target = (short)(p - start);
            else
                *(int64_t *)target = (int64_t)(p - start);
            continue; /* %n never counts as a completed conversion. */
        }
        if (!*p) {
            /* Input failed before the first conversion completed. */
            return completed ? assigned : -1;
        }
        if (strchr("diuoxX", conv)) {
            int base = conv == 'd' || conv == 'u' ? 10 : conv == 'o' ? 8 : conv == 'i' ? 0 : 16;
            int resolved = base;
            char token[512];
            const char *end = collect_int(p, token, sizeof(token), width, base, &resolved);
            if (!end)
                return assigned; /* Matching failure. */
            uint64_t value = (uint64_t)_strtoui64(token, NULL, resolved);
            p = end;
            completed++;
            if (suppress)
                continue;
            store_int(args, value, length, conv == 'd' || conv == 'i');
            if (db_get_errno() == 22)
                return -1; /* A NULL target was rejected inside store_int. */
            assigned++;
        } else if (strchr("aAeEfFgG", conv)) {
            if (length[0] == 'L') {
                db_set_errno(45); /* Long double is rejected. */
                return -1;
            }
            const char *q = skip_spaces(p);
            if (!*q)
                return completed ? assigned : -1; /* Whitespace ate the input. */
            char *limit = NULL;
            errno = 0;
            double value = strtod(q, &limit);
            size_t used = (size_t)(limit - q);
            if (!used) {
                p = q;
                return assigned;
            }
            char token[520];
            if (width && used > (size_t)width) {
                memcpy(token, q, (size_t)width);
                token[width] = 0;
                char *sub_limit = NULL;
                value = strtod(token, &sub_limit);
                used = (size_t)(sub_limit - token);
                if (!used) {
                    p = q;
                    return assigned;
                }
            }
            p = q + used;
            completed++;
            if (suppress)
                continue;
            void *target = (void *)(uintptr_t)integer(args);
            if (!target) {
                db_set_errno(22);
                return -1;
            }
            if (length[0] == 'l')
                *(double *)target = value;
            else
                *(float *)target = (float)value;
            assigned++;
        } else if (conv == 's') {
            const char *q = skip_spaces(p);
            if (!*q)
                return completed ? assigned : -1;
            char *out = NULL;
            if (!suppress) {
                out = (char *)(uintptr_t)integer(args);
                if (!out) {
                    db_set_errno(22);
                    return -1;
                }
            }
            size_t n = 0;
            while (*q && !is_space(*q) && (!width || n + 1 <= (size_t)width)) {
                if (out)
                    out[n] = *q;
                n++;
                q++;
            }
            if (out)
                out[n] = 0;
            p = q;
            completed++;
            if (!suppress)
                assigned++;
        } else if (conv == 'c' || conv == '[') {
            unsigned char set[32] = {0};
            int invert = 0;
            if (conv == '[') {
                if (*fmt == '^') {
                    invert = 1;
                    fmt++;
                }
                if (*fmt == ']') {
                    set[']' >> 3] |= 1 << (']' & 7);
                    fmt++;
                }
                int previous = -1;
                while (*fmt && *fmt != ']') {
                    unsigned char c = (unsigned char)*fmt;
                    if (c == '-' && previous >= 0 && fmt[1] && fmt[1] != ']') {
                        int high = (unsigned char)fmt[1];
                        for (int v = previous; v <= high; v++)
                            set[v >> 3] |= 1 << (v & 7);
                        previous = high;
                        fmt += 2;
                        continue;
                    }
                    set[c >> 3] |= 1 << (c & 7);
                    previous = c;
                    fmt++;
                }
                if (!*fmt) {
                    db_set_errno(45); /* An unterminated scanset is unsupported. */
                    return -1;
                }
                fmt++;
            } else if (length[0]) {
                db_set_errno(45); /* %lc needs wide-character support. */
                return -1;
            }
            char *out = NULL;
            if (!suppress) {
                out = (char *)(uintptr_t)integer(args);
                if (!out) {
                    db_set_errno(22);
                    return -1;
                }
            }
            size_t n = 0;
            while (*p && (!width || n + 1 <= (size_t)width)) {
                int member = (set[(unsigned char)*p >> 3] >> ((unsigned char)*p & 7)) & 1;
                if (conv == '[' ? member == invert : 0)
                    break;
                if (out)
                    out[n] = *p;
                n++;
                p++;
            }
            if (!n)
                return assigned; /* Matching failure for an empty scanset. */
            if (conv == '[' && out)
                out[n] = 0; /* %c copies raw characters without a terminator. */
            completed++;
            if (!suppress)
                assigned++;
        } else if (conv == 'p') {
            int resolved = 16;
            char token[512];
            const char *end = collect_int(p, token, sizeof(token), width, 16, &resolved);
            if (!end)
                return assigned;
            errno = 0;
            uint64_t value = (uint64_t)_strtoui64(token, NULL, 16);
            p = end;
            completed++;
            if (suppress)
                continue;
            void *target = (void *)(uintptr_t)integer(args);
            if (!target) {
                db_set_errno(22);
                return -1;
            }
            *(void **)target = (void *)(uintptr_t)value;
            assigned++;
        } else {
            db_set_errno(45); /* Unrecognized conversion. */
            return -1;
        }
    }
    return assigned;
}
static G int db_vsnprintf(char *out, size_t size, const char *fmt, DbVa *a) {
    return db_snprintf_entry(out, size, fmt, a);
}
static G int db_vsscanf(const char *str, const char *fmt, DbVa *a) {
    return db_sscanf_entry(str, fmt, a);
}
uintptr_t db_format_symbol(const char *name) {
    LOOKUP("_printf", db_guest_printf);
    LOOKUP("_fprintf", db_guest_fprintf);
    LOOKUP("_snprintf", db_guest_snprintf);
    LOOKUP("_vsnprintf", db_vsnprintf);
    LOOKUP("_sscanf", db_guest_sscanf);
    LOOKUP("_vsscanf", db_vsscanf);
    return 0;
}
