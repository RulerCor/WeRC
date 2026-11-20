// Isolated, source-built SILK codec. No Weixin API, files, networking or logging.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <cstdint>
#include <cstdio>
#include <cmath>
#include <cstring>
#include <vector>
#include <array>
#include "SKP_Silk_SDK_API.h"

constexpr uint32_t magic=0x43525657, rate=16000, frame=320, max_samples=rate*60, max_silk=512*1024;
static constexpr unsigned char silk_header[]={2,'#','!','S','I','L','K','_','V','3'};
struct Request {uint32_t magic,version,operation,sample_rate,bytes;};
struct Response {uint32_t magic,version,status,sample_rate,samples,bytes;};
static_assert(sizeof(Request)==20 && sizeof(Response)==24,"codec ABI");
static bool transfer(HANDLE handle,void* bytes,size_t count,bool write) {
    auto p=static_cast<unsigned char*>(bytes);
    while(count){DWORD n=0;const DWORD chunk=static_cast<DWORD>(count);
        if(!(write?WriteFile(handle,p,chunk,&n,nullptr):ReadFile(handle,p,chunk,&n,nullptr)) || !n)return false;
        p+=n;count-=n;
    }return true;
}
static bool encode(const std::vector<int16_t>& pcm,std::vector<unsigned char>& result) {
    if(pcm.empty() || pcm.size()>max_samples || pcm.size()%frame)return false;
    int size=0;if(SKP_Silk_SDK_Get_Encoder_Size(&size) || size<=0 || size>1024*1024)return false;
    std::vector<unsigned char> state(size);SKP_SILK_SDK_EncControlStruct control{};
    if(SKP_Silk_SDK_InitEncoder(state.data(),&control))return false;
    control.API_sampleRate=rate;control.maxInternalSampleRate=rate;control.packetSize=frame;
    control.bitRate=24000;control.packetLossPercentage=0;control.complexity=2;control.useInBandFEC=0;control.useDTX=0;
    result.assign(std::begin(silk_header),std::end(silk_header));
    for(size_t at=0;at<pcm.size();at+=frame){
        std::array<unsigned char,1024> packet{};int16_t bytes=static_cast<int16_t>(packet.size());
        if(SKP_Silk_SDK_Encode(state.data(),&control,pcm.data()+at,frame,packet.data(),&bytes) || bytes<=0 || bytes>1024)return false;
        result.push_back(static_cast<unsigned char>(bytes));result.push_back(static_cast<unsigned char>(bytes>>8));
        result.insert(result.end(),packet.begin(),packet.begin()+bytes);
        if(result.size()>max_silk)return false;
    }
    SecureZeroMemory(state.data(),state.size());return true;
}
static bool decode(const std::vector<unsigned char>& silk,std::vector<int16_t>& pcm) {
    if(silk.size()<=12 || silk.size()>max_silk || std::memcmp(silk.data(),silk_header,sizeof(silk_header)))return false;
    int size=0;if(SKP_Silk_SDK_Get_Decoder_Size(&size) || size<=0 || size>1024*1024)return false;
    std::vector<unsigned char> state(size);if(SKP_Silk_SDK_InitDecoder(state.data()))return false;
    SKP_SILK_SDK_DecControlStruct control{};control.API_sampleRate=rate;pcm.clear();
    for(size_t at=sizeof(silk_header);at<silk.size();){
        if(silk.size()-at<2)return false;size_t bytes=silk[at]|(size_t(silk[at+1])<<8);at+=2;
        if(!bytes || bytes>1024 || bytes>silk.size()-at || pcm.size()>=max_samples)return false;
        // The upstream TOC helper reads an uninitialized nFramesInPacket field.
        // Decode with the initialized SDK state and validate the actual frame
        // count instead; do not run that optional introspection helper.
        std::array<int16_t,960> output{};int16_t samples=static_cast<int16_t>(output.size());
        if(SKP_Silk_SDK_Decode(state.data(),&control,0,silk.data()+at,static_cast<int>(bytes),output.data(),&samples) || samples!=frame || control.moreInternalDecoderFrames || control.framesPerPacket!=1)return false;
        pcm.insert(pcm.end(),output.begin(),output.begin()+samples);at+=bytes;
    }
    SecureZeroMemory(state.data(),state.size());return !pcm.empty();
}
static int self_test() {
    unsigned checks=0;auto check=[&](bool value){if(!value)return false;++checks;return true;};
    std::vector<int16_t> input(rate);for(size_t i=0;i<input.size();++i)input[i]=static_cast<int16_t>(std::sin(double(i)*440*6.283185307179586/rate)*12000);
    std::vector<unsigned char> compressed;std::vector<int16_t> restored;
    if(!check(encode(input,compressed)) || !check(decode(compressed,restored)) || !check(restored.size()==input.size()))return 1;
    double power=0;for(auto s:restored)power+=double(s)*s;
    if(!check(std::sqrt(power/restored.size())>1000))return 1;
    auto invalid=compressed;invalid[0]=0;if(!check(!decode(invalid,restored)))return 1;
    invalid=compressed;invalid.pop_back();if(!check(!decode(invalid,restored)))return 1;
    invalid=compressed;invalid[10]=255;invalid[11]=255;if(!check(!decode(invalid,restored)))return 1;
    input.resize(frame-1);if(!check(!encode(input,invalid)))return 1;
    input.clear();if(!check(!encode(input,invalid)))return 1;
    input.resize(max_samples+frame);if(!check(!encode(input,invalid)))return 1;
    input.assign(max_samples,0);if(!check(encode(input,compressed)) || !check(decode(compressed,restored)) || !check(restored.size()==max_samples))return 1;
    std::printf("{\"checks_passed\":%u,\"source_built_silk\":true,\"real_wechat_accessed\":false,\"messages_sent\":false}\n",checks);return 0;
}
static int run() {
    HANDLE in=GetStdHandle(STD_INPUT_HANDLE),out=GetStdHandle(STD_OUTPUT_HANDLE);
    if(GetFileType(in)!=FILE_TYPE_PIPE || GetFileType(out)!=FILE_TYPE_PIPE)return 2;
    Request r{};Response response{magic,1,1,rate,0,0};std::vector<unsigned char> encoded;std::vector<int16_t> pcm;
    if(!transfer(in,&r,sizeof(r),false))return 2;
    if(r.magic!=magic || r.version!=1 || r.sample_rate!=rate || !r.bytes ||
       (r.operation==1 ? (r.bytes>max_samples*2 || r.bytes%(frame*2)) : (r.operation!=2 || r.bytes>max_silk))) {
        transfer(out,&response,sizeof(response),true);return 2;
    }
    bool success=false;
    if(r.operation==1){pcm.resize(r.bytes/2);if(!transfer(in,pcm.data(),r.bytes,false))return 2;}
    else {encoded.resize(r.bytes);if(!transfer(in,encoded.data(),r.bytes,false))return 2;}
    // One frame, then EOF. A parent must close stdin before awaiting output.
    unsigned char extra=0;DWORD n=0;if(ReadFile(in,&extra,1,&n,nullptr) && n)return 2;
    if(r.operation==1)success=encode(pcm,encoded);else success=decode(encoded,pcm);
    if(success){response.status=0;response.samples=static_cast<uint32_t>(pcm.size());response.bytes=static_cast<uint32_t>(r.operation==1?encoded.size():pcm.size()*2);}
    if(!transfer(out,&response,sizeof(response),true))return 3;
    if(success && !transfer(out,r.operation==1?static_cast<void*>(encoded.data()):static_cast<void*>(pcm.data()),response.bytes,true))return 3;
    if(!pcm.empty())SecureZeroMemory(pcm.data(),pcm.size()*2);if(!encoded.empty())SecureZeroMemory(encoded.data(),encoded.size());
    return success?0:2;
}
int main(int argc,char** argv) {
    try {if(argc==2 && !std::strcmp(argv[1],"--self-test"))return self_test();if(argc!=1)return 2;return run();}
    catch(...){return 3;}
}
