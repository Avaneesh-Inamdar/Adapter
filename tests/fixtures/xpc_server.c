/* XPC listener: register a Mach service, accept one peer, answer a typed
   dictionary request with a reply built through the public libxpc API. */
typedef unsigned long size_t;

typedef void *xpc_object_t;
typedef void *xpc_connection_t;
typedef void (^xpc_handler_t)(xpc_object_t);

struct _xpc_type_s;
extern const struct _xpc_type_s _xpc_type_connection;
extern const struct _xpc_type_s _xpc_type_dictionary;
extern const struct _xpc_type_s _xpc_type_error;

#define XPC_TYPE_CONNECTION ((xpc_object_t) & _xpc_type_connection)
#define XPC_TYPE_DICTIONARY ((xpc_object_t) & _xpc_type_dictionary)
#define XPC_TYPE_ERROR ((xpc_object_t) & _xpc_type_error)

#define XPC_CONNECTION_MACH_SERVICE_LISTENER 1

extern xpc_connection_t xpc_connection_create_mach_service(const char *, void *, unsigned long);
extern void xpc_connection_set_event_handler(xpc_connection_t, xpc_handler_t);
extern void xpc_connection_resume(xpc_connection_t);
extern void xpc_connection_cancel(xpc_connection_t);
extern void xpc_connection_send_message(xpc_connection_t, xpc_object_t);
extern const char *xpc_connection_get_name(xpc_connection_t);
extern xpc_object_t xpc_get_type(xpc_object_t);
extern xpc_object_t xpc_dictionary_create(const char *const *, const xpc_object_t *, size_t);
extern xpc_object_t xpc_dictionary_create_reply(xpc_object_t);
extern void xpc_dictionary_set_string(xpc_object_t, const char *, const char *);
extern void xpc_dictionary_set_int64(xpc_object_t, const char *, long long);
extern void xpc_dictionary_set_bool(xpc_object_t, const char *, int);
extern int xpc_dictionary_get_bool(xpc_object_t, const char *);
extern void xpc_dictionary_set_double(xpc_object_t, const char *, double);
extern void xpc_dictionary_set_value(xpc_object_t, const char *, xpc_object_t);
extern long long xpc_dictionary_get_int64(xpc_object_t, const char *);
extern const char *xpc_dictionary_get_string(xpc_object_t, const char *);
extern size_t xpc_dictionary_get_count(xpc_object_t);
extern xpc_object_t xpc_dictionary_get_array(xpc_object_t, const char *);
extern xpc_object_t xpc_array_create(const xpc_object_t *, size_t);
extern size_t xpc_array_get_count(xpc_object_t);
extern void xpc_array_set_string(xpc_object_t, size_t, const char *);
extern void xpc_array_set_int64(xpc_object_t, size_t, long long);
extern void xpc_release(xpc_object_t);
extern int printf(const char *, ...);
extern int fflush(void *);
extern int open(const char *, int, ...);
extern long write(int, const void *, unsigned long);
extern int close(int);
extern int usleep(unsigned int);

#define O_WRONLY 1
#define O_CREAT 0x200
#define O_TRUNC 0x400

static volatile int peer_invalid;

static int marker(const char *path, const char *text) {
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0)
        return -1;
    unsigned long length = 0;
    while (text[length])
        length++;
    long written = write(fd, text, length);
    close(fd);
    return written == (long)length ? 0 : -1;
}

static void serve(xpc_connection_t peer) {
    xpc_connection_set_event_handler(peer, ^(xpc_object_t event) {
      if (xpc_get_type(event) == XPC_TYPE_DICTIONARY) {
          const char *op = xpc_dictionary_get_string(event, "op");
          long long value = xpc_dictionary_get_int64(event, "value");
          printf("server:message=%s value=%lld count=%lu\n", op ? op : "(null)", value,
                 xpc_dictionary_get_count(event));
          xpc_object_t reply = xpc_dictionary_create_reply(event);
          if (!reply) {
              printf("server:reply=null\n");
              fflush(0);
              return;
          }
          xpc_dictionary_set_string(reply, "echo", op ? op : "");
          xpc_dictionary_set_int64(reply, "pong", value + 1);
          xpc_dictionary_set_bool(reply, "ok", 1);
          xpc_dictionary_set_double(reply, "ratio", 2.5);
          xpc_object_t list = xpc_array_create(0, 0);
          xpc_array_set_string(list, 0, "alpha");
          xpc_array_set_int64(list, 1, 7);
          xpc_dictionary_set_value(reply, "list", list);
          xpc_release(list);
          xpc_object_t nested = xpc_dictionary_create(0, 0, 0);
          xpc_dictionary_set_string(nested, "kind", "fixture");
          xpc_dictionary_set_value(reply, "nested", nested);
          xpc_release(nested);
          xpc_object_t stored = xpc_dictionary_get_array(reply, "list");
          printf("server:reply_check ok=%d list=%lu\n", xpc_dictionary_get_bool(reply, "ok"),
                 stored ? xpc_array_get_count(stored) : 0);
          xpc_connection_send_message(peer, reply);
          xpc_release(reply);
          printf("server:reply=1\n");
          fflush(0);
      } else if (xpc_get_type(event) == XPC_TYPE_ERROR) {
          printf("server:peer_invalid=1\n");
          fflush(0);
          peer_invalid = 1;
      }
    });
    xpc_connection_resume(peer);
}

int main(int argc, char **argv) {
    if (argc != 3)
        return 1;
    const char *service = argv[1];
    const char *ready = argv[2];

    xpc_connection_t listener =
        xpc_connection_create_mach_service(service, 0, XPC_CONNECTION_MACH_SERVICE_LISTENER);
    if (!listener)
        return 2;
    xpc_connection_set_event_handler(listener, ^(xpc_object_t event) {
      if (xpc_get_type(event) == XPC_TYPE_CONNECTION) {
          const char *name = xpc_connection_get_name((xpc_connection_t)event);
          printf("server:accept=%s\n", name ? name : "(null)");
          fflush(0);
          serve((xpc_connection_t)event);
      } else if (xpc_get_type(event) == XPC_TYPE_ERROR) {
          printf("server:listener_error=1\n");
          fflush(0);
      }
    });
    xpc_connection_resume(listener);
    if (marker(ready, "ready\n") != 0)
        return 3;
    printf("server:listener=1 name=%s\n", service);
    fflush(0);

    int waited = 0;
    while (!peer_invalid && waited < 200) {
        usleep(100000);
        waited++;
    }
    printf("server:done=1 invalid=%d\n", peer_invalid);
    fflush(0);
    xpc_connection_cancel(listener);
    xpc_release(listener);
    return peer_invalid ? 0 : 4;
}
