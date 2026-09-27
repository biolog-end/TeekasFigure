package dev.teekasfigure.exporter;
import net.minecraft.client.MinecraftClient;
import net.minecraft.text.MutableText;
import net.minecraft.text.Text;
import java.util.Locale;
public final class UiText {
    public static MutableText text(String key,Object... args) {
        var client=MinecraftClient.getInstance();
        String locale=client==null || client.options==null?Locale.getDefault().getLanguage():client.options.language;
        return Text.translatableWithFallback(key,Localizations.fallback(key,locale),args);
    }
}
