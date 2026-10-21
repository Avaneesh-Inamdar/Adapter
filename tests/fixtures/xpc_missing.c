/* XPC missing-service client: a connection to an unregistered name must
   deliver XPC_ERROR_CONNECTION_INVALID to the event handler. */
typedef void *xpc_object_t;
typedef void *xpc_connection_t;
typedef void (^xpc_handler_t)(xpc_object_t);

struct _xpc_type_s;
extern const struct _xpc_type_s _xpc_type_error;
extern const struct _xpc_type_s _xpc_error_connection_invalid;

#define XPC_TYPE_ERROR ((xpc_object_t) & _xpc_type_error)
#define XPC_ERROR_CONNECTION_INVALID ((xpc_object_t) & _xpc_error_connection_invalid)

extern xpc_connection_t xpc_connection_create_mach_service(const char *, void *, unsigned long);
extern void xpc_connection_set_event_handler(xpc_connection_t, xpc_handler_t);
extern void xpc_connection_resume(xpc_connection_t);
extern void xpc_connection_cancel(xpc_connection_t);
extern xpc_object_t xpc_get_type(xpc_object_t);
extern void xpc_release(xpc_object_t);
extern int printf(const char *, ...);
extern int fflush(void *);
extern int usleep(unsigned int);

static volatile int invalid_seen;

int main(int argc, char **argv) {
    if (argc != 2)
        return 1;
    xpc_connection_t connection = xpc_connection_create_mach_service(argv[1], 0, 0);
    if (!connection)
        return 2;
    xpc_connection_set_event_handler(connection, ^(xpc_object_t event) {
      if (xpc_get_type(event) == XPC_TYPE_ERROR) {
          if (event == XPC_ERROR_CONNECTION_INVALID) {
              printf("missing:invalid=1\n");
              fflush(0);
              invalid_seen = 1;
          } else {
              printf("missing:other_error=1\n");
              fflush(0);
          }
      }
    });
    xpc_connection_resume(connection);
    int waited = 0;
    while (!invalid_seen && waited < 50) {
        usleep(100000);
        waited++;
    }
    printf("missing:waited=%d\n", waited);
    fflush(0);
    xpc_connection_cancel(connection);
    xpc_release(connection);
    if (!invalid_seen)
        return 3;
    printf("xpc:missing service passed\n");
    return 0;
}
