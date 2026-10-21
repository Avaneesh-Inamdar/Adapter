extern int puts(const char *);

__attribute__((constructor)) static void plugin_initialize(void) { puts("plugin:tls-init"); }

__thread int plugin_tls = 7;

int plugin_tls_value(void) { return plugin_tls; }
