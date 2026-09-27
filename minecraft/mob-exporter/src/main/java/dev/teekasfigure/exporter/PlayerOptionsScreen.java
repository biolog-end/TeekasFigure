package dev.teekasfigure.exporter;
import net.minecraft.client.gui.DrawContext;
import net.minecraft.client.gui.screen.Screen;
import net.minecraft.client.gui.widget.ButtonWidget;
public final class PlayerOptionsScreen extends Screen {
    private final PlayerScreen parent;
    private int scroll,contentHeight;
    public PlayerOptionsScreen(PlayerScreen parent){super(UiText.text("teekasfigure.player.options"));this.parent=parent;}
    @Override protected void init() {
        int w=Math.min(360,width-24),left=(width-w)/2;
        addDrawableChild(ButtonWidget.builder(UiText.text(parent.clearOthers?"teekasfigure.player.clean_on":"teekasfigure.player.clean_off"),b -> {parent.clearOthers=!parent.clearOthers;b.setMessage(UiText.text(parent.clearOthers?"teekasfigure.player.clean_on":"teekasfigure.player.clean_off"));}).dimensions(left,50,w,20).build());
        addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.export.back"),b -> close()).dimensions(left,height-28,w,20).build());
    }
    @Override public boolean shouldPause(){return false;}
    @Override public void render(DrawContext context,int x,int y,float delta) {
        super.render(context,x,y,delta);context.drawCenteredTextWithShadow(textRenderer,title,width/2,20,0xffffff);
        int w=Math.min(360,width-24),left=(width-w)/2,lineY=80-scroll;
        context.enableScissor(left,78,left+w,height-36);
        for(String key:new String[]{"teekasfigure.player.clean_warning","teekasfigure.player.profile_info"}) {
            for(var line:textRenderer.wrapLines(UiText.text(key),w)){context.drawTextWithShadow(textRenderer,line,left,lineY,key.contains("warning")?0xffaa55:0xdddddd);lineY+=10;}lineY+=12;
        }
        context.disableScissor();contentHeight=lineY+scroll-80;
        if(contentHeight>height-116)context.fill(left+w+2,78,left+w+4,height-36,0xff666666);
    }
    @Override public boolean mouseScrolled(double x,double y,double horizontal,double vertical){scroll=(int)Math.clamp(scroll-vertical*20,0,Math.max(0,contentHeight-(height-116)));return true;}
    @Override public void close(){client.setScreen(parent);}
}
