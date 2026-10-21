#include "bridge.h"
#include <stdlib.h>
#include <string.h>
#include <limits.h>

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
static G int db_vsnprintf(char *out, size_t size, const char *fmt, DbVa *a) {
    return db_snprintf_entry(out, size, fmt, a);
}
uintptr_t db_format_symbol(const char *name) {
    LOOKUP("_printf", db_guest_printf);
    LOOKUP("_fprintf", db_guest_fprintf);
    LOOKUP("_snprintf", db_guest_snprintf);
    LOOKUP("_vsnprintf", db_vsnprintf);
    return 0;
}
