package dev.teekasfigure.exporter.mixin;

import dev.teekasfigure.exporter.ExportScreen;
import dev.teekasfigure.exporter.PlayerScreen;
import dev.teekasfigure.exporter.UiText;
import net.minecraft.client.gui.screen.GameMenuScreen;
import net.minecraft.client.gui.screen.Screen;
import net.minecraft.client.gui.widget.ButtonWidget;
import net.minecraft.text.Text;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(GameMenuScreen.class)
public abstract class GameMenuMixin extends Screen {
    protected GameMenuMixin(Text title) { super(title); }

    @Inject(method = "initWidgets", at = @At("TAIL"))
    private void teekasfigure$button(CallbackInfo ci) {
        addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.export.open"),
            button -> client.setScreen(new ExportScreen((Screen)(Object)this)))
            .dimensions(8, height - 28, 218, 20).build());
        addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.player.open"),
            button -> client.setScreen(new PlayerScreen((Screen)(Object)this)))
            .dimensions(8,height-52,218,20).build());
    }
}
