package dev.teekasfigure.exporter;

import javax.imageio.ImageIO;
import java.io.IOException;
import java.nio.file.*;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;

/** Conservative alpha footprints: overlapping later shapes stay above earlier ones. */
public final class LayerPacker {
    private static final int MASK=64, STRIDE=MASK+1;
    public record Layout(double[] bottoms,double height) {}
    public static final class Footprint {
        private final int[] sum;
        private Footprint(int[] sum){this.sum=sum;}
        public static Footprint solid(){int[] sum=new int[STRIDE*STRIDE];for(int y=1;y<=MASK;y++)for(int x=1;x<=MASK;x++)sum[y*STRIDE+x]=x*y;return new Footprint(sum);}
        public static Footprint read(Path path)throws IOException {
            if(Files.size(path)>16L*1024*1024)throw new IOException("Brush texture is too large");
            boolean[] cells=new boolean[MASK*MASK];
            try(var input=ImageIO.createImageInputStream(path.toFile())) {
                if(input==null)throw new IOException("Cannot read brush texture: "+path);
                var readers=ImageIO.getImageReaders(input);if(!readers.hasNext())throw new IOException("Unsupported brush texture");
                var reader=readers.next();
                try {
                    reader.setInput(input,true,true);int w=reader.getWidth(0),h=reader.getHeight(0);
                    if(w<=0 || h<=0 || w>2048 || h>2048)throw new IOException("Brush dimensions exceed decoder budget");
                    var image=reader.read(0);
                    for(int y=0;y<h;y++)for(int x=0;x<w;x++)if((image.getRGB(x,y)>>>24)!=0)cells[(y*MASK/h)*MASK+x*MASK/w]=true;
                    image.flush();
                } finally {reader.dispose();}
            }
            return fromCells(cells);
        }
        public static Footprint fromCells(boolean[] cells) {
            if(cells.length!=MASK*MASK)throw new IllegalArgumentException("Expected 64x64 footprint");
            int[] sum=new int[STRIDE*STRIDE];
            for(int y=0;y<MASK;y++)for(int x=0;x<MASK;x++)sum[(y+1)*STRIDE+x+1]=(cells[y*MASK+x]?1:0)+sum[y*STRIDE+x+1]+sum[(y+1)*STRIDE+x]-sum[y*STRIDE+x];
            return new Footprint(sum);
        }
        boolean touches(double u,double v,double radius) {
            int x0=Math.max(0,(int)Math.floor((u-radius)*MASK)),y0=Math.max(0,(int)Math.floor((v-radius)*MASK));
            int x1=Math.min(MASK,(int)Math.ceil((u+radius)*MASK)),y1=Math.min(MASK,(int)Math.ceil((v+radius)*MASK));
            return x0<x1 && y0<y1 && sum[y1*STRIDE+x1]-sum[y0*STRIDE+x1]-sum[y1*STRIDE+x0]+sum[y0*STRIDE+x0]>0;
        }
    }
    public static Layout pack(List<SceneTimeline.Shape> shapes,SceneTimeline.Brush[] brushes,Footprint[] masks,double width,double height,AtomicBoolean cancelled)throws IOException {
        double cell=Math.max(1,Math.max(width,height)/128);
        int columns=(int)Math.ceil(width/cell),rows=(int)Math.ceil(height/cell);
        double[] tops=new double[columns*rows],bottoms=new double[shapes.size()];int[] covered=new int[tops.length];
        double highest=0;
        for(int i=0;i<shapes.size();i++) {
            if(cancelled.get())throw new IOException("Cancelled");
            var q=shapes.get(i);var brush=brushes[q.brush()];var mask=masks[q.brush()];
            double cos=Math.cos(q.rotation()),sin=Math.sin(q.rotation()),extent=q.size()*(Math.abs(cos)+Math.abs(sin))/2;
            int left=Math.max(0,(int)Math.floor((q.x()-extent)/cell)),right=Math.min(columns,(int)Math.ceil((q.x()+extent)/cell));
            int top=Math.max(0,(int)Math.floor((q.y()-extent)/cell)),bottom=Math.min(rows,(int)Math.ceil((q.y()+extent)/cell));
            // Inverse-transform the whole cell, rather than sampling its center:
            // even a one-pixel fin or a rotated edge contributes conservatively.
            double radius=cell*(Math.abs(cos)+Math.abs(sin))/2/q.size(),base=0;int n=0;
            for(int y=top;y<bottom;y++)for(int x=left;x<right;x++) {
                double dx=(x+.5)*cell-q.x(),dy=(y+.5)*cell-q.y();
                if(mask.touches((cos*dx+sin*dy)/q.size()+.5,(-sin*dx+cos*dy)/q.size()+.5,radius)) {
                    int index=y*columns+x;covered[n++]=index;base=Math.max(base,tops[index]);
                }
            }
            bottoms[i]=base;double roof=base+brush.depth()*q.size()/brush.span()+2;
            for(int c=0;c<n;c++)tops[covered[c]]=roof;
            highest=Math.max(highest,roof);
        }
        return new Layout(bottoms,highest);
    }
}
