/* XPC client: connect to a named Mach service, send one typed dictionary and
   validate the typed reply through the public libxpc API. */
typedef unsigned long size_t;

typedef void *xpc_object_t;
typedef void *xpc_connection_t;
typedef void (^xpc_handler_t)(xpc_object_t);

struct _xpc_type_s;
extern const struct _xpc_type_s _xpc_type_dictionary;
extern const struct _xpc_type_s _xpc_type_array;
extern const struct _xpc_type_s _xpc_type_error;
extern const struct _xpc_type_s _xpc_error_connection_invalid;
extern const struct _xpc_type_s _xpc_error_connection_interrupted;

#define XPC_TYPE_DICTIONARY ((xpc_object_t) & _xpc_type_dictionary)
#define XPC_TYPE_ARRAY ((xpc_object_t) & _xpc_type_array)
#define XPC_TYPE_ERROR ((xpc_object_t) & _xpc_type_error)
#define XPC_ERROR_CONNECTION_INVALID ((xpc_object_t) & _xpc_error_connection_invalid)
#define XPC_ERROR_CONNECTION_INTERRUPTED ((xpc_object_t) & _xpc_error_connection_interrupted)

extern xpc_connection_t xpc_connection_create_mach_service(const char *, void *, unsigned long);
extern void xpc_connection_set_event_handler(xpc_connection_t, xpc_handler_t);
extern void xpc_connection_resume(xpc_connection_t);
extern void xpc_connection_cancel(xpc_connection_t);
extern xpc_object_t xpc_connection_send_message_with_reply_sync(xpc_connection_t, xpc_object_t);
extern xpc_object_t xpc_get_type(xpc_object_t);
extern xpc_object_t xpc_dictionary_create(const char *const *, const xpc_object_t *, size_t);
extern void xpc_dictionary_set_string(xpc_object_t, const char *, const char *);
extern void xpc_dictionary_set_int64(xpc_object_t, const char *, long long);
extern void xpc_dictionary_set_bool(xpc_object_t, const char *, int);
extern void xpc_dictionary_set_double(xpc_object_t, const char *, double);
extern long long xpc_dictionary_get_int64(xpc_object_t, const char *);
extern int xpc_dictionary_get_bool(xpc_object_t, const char *);
extern double xpc_dictionary_get_double(xpc_object_t, const char *);
extern const char *xpc_dictionary_get_string(xpc_object_t, const char *);
extern xpc_object_t xpc_dictionary_get_value(xpc_object_t, const char *);
extern xpc_object_t xpc_dictionary_get_array(xpc_object_t, const char *);
extern xpc_object_t xpc_dictionary_get_dictionary(xpc_object_t, const char *);
extern size_t xpc_dictionary_get_count(xpc_object_t);
extern size_t xpc_array_get_count(xpc_object_t);
extern const char *xpc_array_get_string(xpc_object_t, size_t);
extern long long xpc_array_get_int64(xpc_object_t, size_t);
extern void xpc_release(xpc_object_t);
extern int printf(const char *, ...);
extern int fflush(void *);
extern int strcmp(const char *, const char *);
extern int usleep(unsigned int);

static volatile int invalid_seen;

static int check_double(double actual, double expected) {
    double difference = actual - expected;
    return difference < 0 ? -difference < 1e-9 : difference < 1e-9;
}

int main(int argc, char **argv) {
    if (argc != 2)
        return 1;
    xpc_connection_t connection = xpc_connection_create_mach_service(argv[1], 0, 0);
    if (!connection)
        return 2;
    xpc_connection_set_event_handler(connection, ^(xpc_object_t event) {
      if (xpc_get_type(event) == XPC_TYPE_ERROR) {
          if (event == XPC_ERROR_CONNECTION_INVALID) {
              printf("client:invalid=1\n");
              fflush(0);
              invalid_seen = 1;
          } else if (event == XPC_ERROR_CONNECTION_INTERRUPTED) {
              printf("client:interrupted=1\n");
              fflush(0);
          }
      } else {
          printf("client:unexpected_event=1\n");
          fflush(0);
      }
    });
    xpc_connection_resume(connection);

    xpc_object_t request = xpc_dictionary_create(0, 0, 0);
    xpc_dictionary_set_string(request, "op", "ping");
    xpc_dictionary_set_int64(request, "value", 41);
    xpc_dictionary_set_bool(request, "flag", 1);
    xpc_dictionary_set_double(request, "ratio", 2.5);
    xpc_object_t reply = xpc_connection_send_message_with_reply_sync(connection, request);
    xpc_release(request);
    if (!reply) {
        printf("client:reply=null\n");
        return 3;
    }
    if (xpc_get_type(reply) == XPC_TYPE_ERROR) {
        printf("client:reply=error\n");
        xpc_release(reply);
        return 4;
    }
    if (xpc_get_type(reply) != XPC_TYPE_DICTIONARY) {
        printf("client:reply=wrong_type\n");
        xpc_release(reply);
        return 5;
    }
    const char *echo = xpc_dictionary_get_string(reply, "echo");
    long long pong = xpc_dictionary_get_int64(reply, "pong");
    int ok = xpc_dictionary_get_bool(reply, "ok");
    double ratio = xpc_dictionary_get_double(reply, "ratio");
    size_t count = xpc_dictionary_get_count(reply);
    xpc_object_t list = xpc_dictionary_get_array(reply, "list");
    xpc_object_t nested = xpc_dictionary_get_dictionary(reply, "nested");
    const char *kind = nested ? xpc_dictionary_get_string(nested, "kind") : 0;
    printf("client:reply_type=dictionary\n");
    printf("client:echo=%s\n", echo ? echo : "(null)");
    printf("client:pong=%lld ok=%d ratio=%.1f count=%lu\n", pong, ok, ratio, count);
    printf("client:list=%lu first=%s second=%lld\n", list ? xpc_array_get_count(list) : 0,
           list ? xpc_array_get_string(list, 0) : "(null)",
           list ? xpc_array_get_int64(list, 1) : -1);
    printf("client:nested=%s\n", kind ? kind : "(null)");
    int valid = echo && strcmp(echo, "ping") == 0 && pong == 42 && ok == 1 &&
                check_double(ratio, 2.5) && count == 6 && list && xpc_array_get_count(list) == 2 &&
                strcmp(xpc_array_get_string(list, 0), "alpha") == 0 &&
                xpc_array_get_int64(list, 1) == 7 && kind && strcmp(kind, "fixture") == 0;
    printf("client:valid=%d\n", valid);
    fflush(0);
    xpc_release(reply);
    xpc_connection_cancel(connection);

    int waited = 0;
    while (!invalid_seen && waited < 50) {
        usleep(100000);
        waited++;
    }
    printf("client:released=1\n");
    fflush(0);
    xpc_release(connection);
    if (!valid)
        return 6;
    if (!invalid_seen)
        return 7;
    printf("xpc:client reply passed\n");
    return 0;
}
