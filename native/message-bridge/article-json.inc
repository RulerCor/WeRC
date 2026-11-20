// Read-only projection of the pinned core's 16-byte JSON values. Layout verified
// against its parser/destructor; never transfers ownership or calls an allocator.
namespace article_native {
constexpr size_t MAX_RESPONSE=2*1024*1024;
static bool read(void* out,uintptr_t address,size_t size) {
    __try {if(!address)return false;std::memcpy(out,reinterpret_cast<const void*>(address),size);return true;}
    __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
template<class T> static T read_value(uintptr_t address) {
    T value{};if(!read(&value,address,sizeof(value)))throw std::runtime_error("native article layout");return value;
}
static std::string small(uintptr_t address,size_t max=MAX_RESPONSE) {
    const auto s=read_value<SmallString>(address);
    if(s.length>max || s.capacity<s.length || (s.capacity!=15&&s.capacity<16) || s.capacity>16*1024*1024)throw std::runtime_error("native article string");
    std::string out(size_t(s.length),'\0');
    const auto source=s.capacity==15?address:read_value<uintptr_t>(address);
    if(s.length&&!read(out.data(),source,size_t(s.length)))throw std::runtime_error("native article string");
    return out;
}
static std::string quoted(const std::string& value) {
    std::string out="\"";
    for(unsigned char c:value){
        if(c=='"'||c=='\\'){out+='\\';out+=char(c);}
        else if(c<32){const char h[]="0123456789abcdef";out+="\\u00";out+=h[c>>4];out+=h[c&15];}
        else out+=char(c);
        if(out.size()>MAX_RESPONSE)throw std::runtime_error("native article size");
    }
    return out+'"';
}
struct JsonProjection {
    std::string out;size_t nodes=0;
    void add(const std::string& text){if(text.size()>MAX_RESPONSE-out.size())throw std::runtime_error("native article size");out+=text;}
    void value(uintptr_t address,unsigned depth=0){
        if(depth>64||++nodes>32768)throw std::runtime_error("native article depth");
        const auto type=read_value<uint8_t>(address);const auto data=read_value<uint64_t>(address+8);
        switch(type){
        case 0:add("null");break;
        case 1:{
            const auto head=read_value<uintptr_t>(data),count=read_value<uint64_t>(data+8);
            if(count>32768 || read_value<uint8_t>(head+0x19)!=1)throw std::runtime_error("native article map");
            uintptr_t node=read_value<uintptr_t>(head);std::set<uintptr_t> visited;add("{");
            while(node!=head){
                if(visited.size()>=count||!visited.insert(node).second||read_value<uint8_t>(node+0x19))throw std::runtime_error("native article map");
                if(visited.size()>1)add(",");add(quoted(small(node+0x20)));add(":");value(node+0x40,depth+1);
                auto right=read_value<uintptr_t>(node+0x10);size_t walk=0;
                if(!read_value<uint8_t>(right+0x19)){
                    node=right;
                    while(!read_value<uint8_t>(read_value<uintptr_t>(node)+0x19)){if(++walk>count)throw std::runtime_error("native article map cycle");node=read_value<uintptr_t>(node);}
                } else {
                    auto parent=read_value<uintptr_t>(node+8);
                    while(!read_value<uint8_t>(parent+0x19)&&node==read_value<uintptr_t>(parent+0x10)){if(++walk>count)throw std::runtime_error("native article map cycle");node=parent;parent=read_value<uintptr_t>(parent+8);}
                    node=parent;
                }
            }
            if(visited.size()!=count)throw std::runtime_error("native article map size");add("}");break;
        }
        case 2:{
            const auto begin=read_value<uintptr_t>(data),end=read_value<uintptr_t>(data+8),capacity=read_value<uintptr_t>(data+16);
            if(end<begin||capacity<end||(end-begin)%16||(end-begin)/16>32768)throw std::runtime_error("native article array");
            add("[");for(auto at=begin;at<end;at+=16){if(at!=begin)add(",");value(at,depth+1);}add("]");break;
        }
        case 3:add(quoted(small(data)));break;
        case 4:if(uint8_t(data)>1)throw std::runtime_error("native article boolean");add(uint8_t(data)?"true":"false");break;
        case 5:add(std::to_string(static_cast<int64_t>(data)));break;
        case 6:add(std::to_string(data));break;
        case 7:{double v;std::memcpy(&v,&data,8);if(!std::isfinite(v))throw std::runtime_error("native article number");char text[64];auto r=std::to_chars(text,text+sizeof(text),v);if(r.ec!=std::errc{})throw std::runtime_error("native article number");add(std::string(text,r.ptr));break;}
        default:throw std::runtime_error("native article type");
        }
    }
};
}
