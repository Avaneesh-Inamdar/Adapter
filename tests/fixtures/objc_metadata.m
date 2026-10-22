/* Compiler-emitted protocols/properties; also built unchanged by the Mac job. */
typedef signed char Bool;
typedef void *Property;
typedef struct {
    SEL name;
    const char *types;
} MethodDescription;
extern int printf(const char *, ...), strcmp(const char *, const char *), puts(const char *);
extern Class objc_getClass(const char *), object_getClass(id);
extern Protocol *objc_getProtocol(const char *);
extern const char *protocol_getName(Protocol *), *property_getName(Property),
    *property_getAttributes(Property);
extern Bool protocol_isEqual(Protocol *, Protocol *),
    protocol_conformsToProtocol(Protocol *, Protocol *);
extern Bool class_conformsToProtocol(Class, Protocol *);
extern Property class_getProperty(Class, const char *),
    protocol_getProperty(Protocol *, const char *, Bool, Bool);
extern MethodDescription protocol_getMethodDescription(Protocol *, SEL, Bool, Bool);
extern SEL sel_registerName(const char *);
extern id class_createInstance(Class, unsigned long), object_dispose(id);

@protocol MetadataBase
@property(nonatomic, readonly) long score;
- (long)required:(long)value;
@optional
- (long)optional:(long)value;
+ (long)classOptional;
@end
@protocol MetadataDerived <MetadataBase>
+ (long)classRequired;
@property(class, readonly) long total;
@end
@protocol MetadataExtra
- (long)extra;
@end

__attribute__((objc_root_class))
@interface MetadataRoot<MetadataDerived> {
    Class _isa;
    long _score;
}
@property(nonatomic, assign) long score;
+ (long)classRequired;
+ (long)total;
- (long)required:(long)value;
@end
@implementation MetadataRoot
@synthesize score = _score;
+ (long)classRequired {
    return 23;
}
+ (long)total {
    return 41;
}
- (long)required:(long)value {
    return value * 3;
}
@end
@interface MetadataChild : MetadataRoot
@end
@implementation MetadataChild
@end
@interface MetadataRoot (Extras) <MetadataExtra>
@property(nonatomic, readonly) long categoryScore;
@property(class, readonly) long categoryTotal;
@end
@implementation MetadataRoot (Extras)
- (long)extra {
    return 19;
}
- (long)categoryScore {
    return 17;
}
+ (long)categoryTotal {
    return 29;
}
@end

static unsigned long hash(const char *text) {
    if (!text)
        return 0;
    unsigned long value = 2166136261UL;
    while (*text)
        value = ((value ^ (unsigned char)*text++) * 16777619UL) & 0xffffffffUL;
    return value;
}
#ifndef DIFF_CASE_COUNT
__attribute__((constructor)) static void metadata_constructor(void) {
    puts("objc:metadata constructor");
}
#endif
int main(void) {
    Class root = objc_getClass("MetadataRoot"), child = objc_getClass("MetadataChild");
    Protocol *base = @protocol(MetadataBase), *derived = @protocol(MetadataDerived),
             *extra = @protocol(MetadataExtra);
    static const char *families[8] = {"identity",         "conformance",      "classproperty",
                                      "categoryproperty", "protocolproperty", "required",
                                      "optional",         "accessors"};
#ifndef DIFF_CASE_COUNT
#define DIFF_CASE_COUNT 8
#define METADATA_LOCAL_CHECK
#endif
    for (int index = 0; index < DIFF_CASE_COUNT; index++) {
        long results[4] = {0};
        Property property;
        MethodDescription description;
        switch (index % 8) {
        case 0:
            results[0] = base == objc_getProtocol("MetadataBase");
            results[1] = hash(protocol_getName(derived));
            results[2] = protocol_isEqual(base, derived);
            results[3] = protocol_isEqual((Protocol *)0, (Protocol *)0);
            break;
        case 1:
            results[0] = protocol_conformsToProtocol(derived, base);
            results[1] = protocol_conformsToProtocol(base, derived);
            results[2] = class_conformsToProtocol(root, extra);
            results[3] = class_conformsToProtocol(child, base);
            break;
        case 2:
            property = class_getProperty(child, "score");
            results[0] = property != (void *)0;
            results[1] = hash(property_getName(property));
            results[2] = hash(property_getAttributes(property));
            results[3] = class_getProperty(root, "absent") == (void *)0;
            break;
        case 3:
            property = class_getProperty(child, "categoryScore");
            results[0] = hash(property_getAttributes(property));
            property = class_getProperty(object_getClass((id)root), "categoryTotal");
            results[1] = hash(property_getName(property));
            results[2] = hash(property_getAttributes(property));
            results[3] = class_conformsToProtocol(root, base);
            break;
        case 4:
            property = protocol_getProperty(derived, "score", 1, 1);
            results[0] = hash(property_getAttributes(property));
            property = protocol_getProperty(derived, "total", 1, 0);
            results[1] = hash(property_getAttributes(property));
            results[2] = protocol_getProperty(derived, "score", 0, 1) == (void *)0;
            results[3] = protocol_getProperty((Protocol *)0, "score", 1, 1) == (void *)0;
            break;
        case 5:
            description =
                protocol_getMethodDescription(derived, sel_registerName("required:"), 1, 1);
            results[0] = description.name == sel_registerName("required:");
            results[1] = hash(description.types);
            description =
                protocol_getMethodDescription(derived, sel_registerName("classRequired"), 1, 0);
            results[2] = description.name != (SEL)0;
            results[3] = hash(description.types);
            break;
        case 6:
            description =
                protocol_getMethodDescription(derived, sel_registerName("optional:"), 0, 1);
            results[0] = description.name != (SEL)0;
            results[1] = hash(description.types);
            description =
                protocol_getMethodDescription(derived, sel_registerName("optional:"), 1, 1);
            results[2] = description.name == (SEL)0 && description.types == (void *)0;
            results[3] = objc_getProtocol("NoSuchMetadataProtocol") == (void *)0;
            break;
        default: {
            MetadataRoot *object = class_createInstance(root, 0);
            object.score = index * 7;
            results[0] = object.score;
            results[1] = [object categoryScore];
            results[2] = [MetadataRoot categoryTotal];
            results[3] = [object required:index];
            object_dispose(object);
            break;
        }
        }
#ifdef METADATA_LOCAL_CHECK
        if ((index == 0 && (!results[0] || results[2] || !results[3])) ||
            (index == 1 && (!results[0] || results[1] || !results[2] || results[3])) ||
            (index == 2 && (!results[0] || !results[1] || !results[2] || !results[3])) ||
            (index == 3 && (!results[0] || !results[1] || !results[2] || !results[3])) ||
            (index == 4 && (!results[0] || !results[1] || !results[2] || !results[3])) ||
            (index == 5 && (!results[0] || !results[1] || !results[2] || !results[3])) ||
            (index == 6 && (!results[0] || !results[1] || !results[2] || !results[3])) ||
            (index == 7 &&
             (results[0] != 49 || results[1] != 17 || results[2] != 29 || results[3] != 21)))
            return 10 + index;
#else
        printf("{\"id\":\"%s:%d\",\"family\":\"%s\",\"index\":%d,\"result\":[%ld,%ld,%ld,%ld]}\n",
               families[index % 8], index, families[index % 8], index, results[0], results[1],
               results[2], results[3]);
#endif
    }
#ifdef METADATA_LOCAL_CHECK
    puts("objc:protocol inheritance, descriptions, properties and accessors passed");
#endif
    return 0;
}
