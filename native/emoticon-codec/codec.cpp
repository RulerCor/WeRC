// SPDX-License-Identifier: GPL-3.0-or-later
// Standalone bounded HEVC + JBIG frame decoder. No WeChat/API code is linked.
#include <libde265/de265.h>
extern "C" {
#include "jbig.h"
}
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <iostream>
#include <vector>
#include <fcntl.h>
#include <io.h>
namespace {
constexpr uint32_t MAGIC=0x45525657, MAX_BYTES=1024*1024;
struct Bytes:std::vector<uint8_t>{using std::vector<uint8_t>::vector;~Bytes(){volatile uint8_t* p=data();for(size_t i=0;i<size();i++)p[i]=0;}};
uint32_t read_word(){uint32_t n=0;std::cin.read(reinterpret_cast<char*>(&n),4);if(!std::cin)throw 1;return n;}
void write_word(uint32_t n){std::cout.write(reinterpret_cast<char*>(&n),4);if(!std::cout)throw 2;}
struct Frame {Bytes mask,picture;};
struct Jbig {jbg_dec_state state;Jbig(){jbg_dec_init(&state);jbg_dec_maxsize(&state,1024,1024);state.maxmem=4*1024*1024;}~Jbig(){jbg_dec_free(&state);}};
struct Decoder {
 de265_decoder_context* ctx=de265_new_decoder();
 uint32_t width,height,count,decoded=0;const std::vector<Frame>& frames;
 Decoder(uint32_t w,uint32_t h,const std::vector<Frame>& f):width(w),height(h),count(static_cast<uint32_t>(f.size())),frames(f){
  if(!ctx)throw 3;auto limits=*de265_get_security_limits(ctx);limits.max_image_size_pixels=1024*1024;limits.max_NAL_size_bytes=MAX_BYTES;limits.max_SEI_messages=64;de265_set_security_limits(ctx,&limits);de265_set_parameter_bool(ctx,DE265_DECODER_PARAM_SUPPRESS_FAULTY_PICTURES,1);
 }
 ~Decoder(){de265_free_decoder(ctx);}
 static uint8_t clamp(int n){return static_cast<uint8_t>(std::clamp(n,0,255));}
 void frame(const de265_image* image){
  if(decoded>=count||de265_get_image_width(image,0)!=static_cast<int>(width)||de265_get_image_height(image,0)!=static_cast<int>(height)||de265_get_chroma_format(image)!=de265_chroma_420)throw 4;
  int strides[3];const uint8_t* planes[3];for(int c=0;c<3;c++){if(de265_get_bits_per_pixel(image,c)!=8)throw 5;planes[c]=de265_get_image_plane(image,c,&strides[c]);if(!planes[c]||strides[c]<de265_get_image_width(image,c))throw 6;}
  const int matrix=de265_get_image_matrix_coefficients(image);if(matrix!=1&&matrix!=2&&matrix!=5&&matrix!=6)throw 7;
  const bool full=de265_get_image_full_range_flag(image)!=0,bt709=matrix==1;
  Bytes rgba(static_cast<size_t>(width)*height*4);const auto& mask=frames[decoded].mask;
  for(uint32_t y=0;y<height;y++)for(uint32_t x=0;x<width;x++){
   int yy=planes[0][y*strides[0]+x],u=int(planes[1][(y/2)*strides[1]+x/2])-128,v=int(planes[2][(y/2)*strides[2]+x/2])-128;
   int r,g,b;if(full){r=yy+(((bt709?403:359)*v+128)>>8);g=yy-(((bt709?48:88)*u+(bt709?120:183)*v+128)>>8);b=yy+(((bt709?475:454)*u+128)>>8);}else{int c=298*(yy-16);r=(c+(bt709?459:409)*v+128)>>8;g=(c-(bt709?55:100)*u-(bt709?136:208)*v+128)>>8;b=(c+(bt709?541:516)*u+128)>>8;}
   size_t at=(static_cast<size_t>(y)*width+x)*4;auto alpha=(mask[y*((width+7)/8)+x/8]&(128>>(x%8)))?255:0;
   rgba[at]=alpha?clamp(r):0;rgba[at+1]=alpha?clamp(g):0;rgba[at+2]=alpha?clamp(b):0;rgba[at+3]=static_cast<uint8_t>(alpha);
  }
  std::cout.write(reinterpret_cast<const char*>(rgba.data()),rgba.size());if(!std::cout)throw 8;decoded++;
 }
 void drain(){for(int steps=0;steps<4096;steps++){int more=0;auto err=de265_decode(ctx,&more);while(auto img=de265_peek_next_picture(ctx)){frame(img);de265_release_next_picture(ctx);}if(de265_get_warning(ctx)!=DE265_OK)throw 9;if(err==DE265_ERROR_WAITING_FOR_INPUT_DATA)return;if(err!=DE265_OK&&err!=DE265_ERROR_IMAGE_BUFFER_FULL)throw 10;if(!more)return;}throw 11;}
 void run(){for(size_t n=0;n<frames.size();n++){const auto& p=frames[n].picture;if(p.empty())continue;if(de265_push_data(ctx,p.data(),static_cast<int>(p.size()),n,nullptr)!=DE265_OK)throw 12;drain();}if(de265_flush_data(ctx)!=DE265_OK)throw 13;drain();if(decoded!=count)throw 14;}
};
}
int main(){try{
 _setmode(_fileno(stdin),_O_BINARY);_setmode(_fileno(stdout),_O_BINARY);
 if(read_word()!=MAGIC||read_word()!=1)return 2;auto width=read_word(),height=read_word(),count=read_word();
 if(width==0||height==0||width>1024||height>1024||count==0||count>120||uint64_t(width)*height*count>48'000'000)return 3;
 std::vector<Frame> frames(count);size_t total=0;
 for(auto& frame:frames){
  auto n=read_word();if(n<20||n>MAX_BYTES||total+n>MAX_BYTES)return 4;total+=n;Bytes input(n);std::cin.read(reinterpret_cast<char*>(input.data()),n);if(!std::cin)return 5;
  // Accept the independently verified single-resolution, single-plane profile.
  if(input[0]!=0||input[1]!=0||input[2]!=1||input[3]!=0)return 6;
  Jbig jbig;size_t used=0;int e=jbg_dec_in(&jbig.state,input.data(),input.size(),&used);
  if(e!=JBG_EOK||used!=input.size()||jbg_dec_getwidth(&jbig.state)!=width||jbg_dec_getheight(&jbig.state)!=height||jbg_dec_getplanes(&jbig.state)!=1)return 7;
  const auto mask=jbg_dec_getimage(&jbig.state,0);if(!mask)return 8;frame.mask.assign(mask,mask+((width+7)/8)*height);
  n=read_word();if(n>MAX_BYTES||total+n>MAX_BYTES)return 9;total+=n;frame.picture.resize(n);std::cin.read(reinterpret_cast<char*>(frame.picture.data()),n);if(!std::cin)return 10;
 }
 if(std::cin.get()!=std::char_traits<char>::eof())return 11;
 for(auto n:{MAGIC,1u,width,height,count})write_word(n);
 Decoder decoder(width,height,frames);decoder.run();write_word(MAGIC);write_word(count);std::cout.flush();return std::cout?0:12;
 }catch(...){return 1;}}
