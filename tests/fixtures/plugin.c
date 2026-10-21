extern int puts(const char *);

int plugin_data = 41;

__attribute__((constructor)) static void plugin_initialize(void) { puts("plugin:init"); }

__attribute__((destructor)) static void plugin_terminate(void) { puts("plugin:fini"); }

int plugin_add(int a, int b) { return a + b; }

const char *plugin_name(void) { return "adapter-plugin"; }
