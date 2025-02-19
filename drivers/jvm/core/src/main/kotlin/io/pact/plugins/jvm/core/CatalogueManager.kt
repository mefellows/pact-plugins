package io.pact.plugins.jvm.core

import au.com.dius.pact.core.model.ContentType
import io.pact.plugin.Plugin
import mu.KLogging
import java.lang.IllegalArgumentException

/**
 * The catalogue manager stores all the entries from the core Pact framework as well as all the loaded plugins
 */
object CatalogueManager : KLogging() {
  private val catalogue = mutableMapOf<String, CatalogueEntry>()

  /**
   * Register the list of entries against the plugin name. Each entry will be keyed by
   * plugin/<plugin-name>/<entry-type>/<entry-key>
   */
  fun registerPluginEntries(name: String, catalogueList: List<Plugin.CatalogueEntry>) {
    catalogueList.forEach {
      val type = CatalogueEntryType.fromEntry(it.type)
      val key = "plugin/$name/${type}/${it.key}"
      catalogue[key] = CatalogueEntry(type, CatalogueEntryProviderType.PLUGIN, name, it.key, it.valuesMap)
    }

    logger.debug { "Updated catalogue entries:\n${catalogue.keys.joinToString("\n")}" }
  }

  /**
   * INTERNAL: Register the entries as core Pact framework entries
   */
  fun registerCoreEntries(entries: List<CatalogueEntry>) {
    entries.forEach {
      val key = "core/${it.type}/${it.key}"
      catalogue[key] = it
    }

    logger.debug { "Core catalogue entries:\n${catalogue.keys.joinToString("\n")}" }
  }

  /**
   * Return all the entries from the catalogue
   */
  fun entries() = catalogue.entries

  /**
   * Lookup entry by key. Entries are keyed by <core|plugin>/<plugin-name>?/<entry-type>/<entry-key>
   */
  fun lookupEntry(key: String): CatalogueEntry? {
    return catalogue[key]
  }

  /**
   * Lookup a content matcher in the catalogue that can handle the given content type
   */
  fun findContentMatcher(contentType: ContentType): ContentMatcher? {
    val catalogueEntry = catalogue.values.find { entry ->
      if (entry.type == CatalogueEntryType.CONTENT_MATCHER) {
        val contentTypes = entry.values["content-types"]?.split(';')
        if (contentTypes.isNullOrEmpty()) {
          false
        } else {
          contentTypes.any { contentType.matches(it) }
        }
      } else {
        false
      }
    }
    return if (catalogueEntry != null)
      CatalogueContentMatcher(catalogueEntry)
      else null
  }

  /**
   * Lookup the content generator the can handle the given content type
   */
  fun findContentGenerator(contentType: ContentType): ContentGenerator? {
    val catalogueEntry = catalogue.values.find { entry ->
      if (entry.type == CatalogueEntryType.CONTENT_GENERATOR) {
        val contentTypes = entry.values["content-types"]?.split(';')
        if (contentTypes.isNullOrEmpty()) {
          false
        } else {
          contentTypes.any { contentType.matches(it) }
        }
      } else {
        false
      }
    }
    return if (catalogueEntry != null)
      CatalogueContentGenerator(catalogueEntry)
    else null
  }

  // TODO
  // /// Remove entries for a plugin
  //pub fn remove_plugin_entries(name: &String) {
  //  let prefix = format!("plugin/{}/", name);
  //  let keys: Vec<String> = {
  //    let guard = CATALOGUE_REGISTER.lock().unwrap();
  //    guard.keys()
  //      .filter(|key| key.starts_with(&prefix))
  //      .cloned()
  //      .collect()
  //  };
  //
  //  let mut guard = CATALOGUE_REGISTER.lock().unwrap();
  //  for key in keys {
  //    guard.remove(&key);
  //  }
  //
  //  debug!("Removed all catalogue entries for plugin {}", name);
  //}
}

private fun ContentType.matches(type: String) = this.getBaseType().orEmpty().matches(Regex(type))

/**
 * Type of entry in the catalogue
 */
enum class CatalogueEntryType {
  CONTENT_MATCHER, CONTENT_GENERATOR, MOCK_SERVER, MATCHER, INTERACTION;

  override fun toString(): String {
    return when (this) {
      CONTENT_MATCHER -> "content-matcher"
      CONTENT_GENERATOR -> "content-generator"
      MOCK_SERVER -> "mock-server"
      MATCHER -> "matcher"
      INTERACTION -> "interaction"
    }
  }

  /**
   * Convert this entry type to the matching Protobuf type
   */
  fun toEntry(): Plugin.CatalogueEntry.EntryType {
    return when (this) {
      CONTENT_MATCHER -> Plugin.CatalogueEntry.EntryType.CONTENT_MATCHER
      CONTENT_GENERATOR -> Plugin.CatalogueEntry.EntryType.CONTENT_GENERATOR
      MOCK_SERVER -> Plugin.CatalogueEntry.EntryType.MOCK_SERVER
      MATCHER -> Plugin.CatalogueEntry.EntryType.MATCHER
      INTERACTION -> Plugin.CatalogueEntry.EntryType.INTERACTION
    }
  }

  companion object {
    /**
     * Return the corresponding entry type from the given string value
     */
    fun fromString(type: String): CatalogueEntryType {
      return when (type) {
        "content-matcher" -> CONTENT_MATCHER
        "content-generator" -> CONTENT_GENERATOR
        "interaction" -> INTERACTION
        "matcher" -> MATCHER
        "mock-server" -> MOCK_SERVER
        else -> throw IllegalArgumentException("'$type' is not a valid CatalogueEntryType value")
      }
    }

    /**
     * Return the catalogue entry type from the corresponding Protobuf entry type
     */
    fun fromEntry(type: Plugin.CatalogueEntry.EntryType?): CatalogueEntryType {
      return if (type != null) {
        when (type) {
          Plugin.CatalogueEntry.EntryType.CONTENT_MATCHER -> CONTENT_MATCHER
          Plugin.CatalogueEntry.EntryType.CONTENT_GENERATOR -> CONTENT_GENERATOR
          Plugin.CatalogueEntry.EntryType.MOCK_SERVER -> MOCK_SERVER
          Plugin.CatalogueEntry.EntryType.MATCHER -> MATCHER
          Plugin.CatalogueEntry.EntryType.INTERACTION -> INTERACTION
          Plugin.CatalogueEntry.EntryType.UNRECOGNIZED -> CONTENT_MATCHER
        }
      } else {
        CONTENT_MATCHER
      }
    }
  }
}

/**
 * Entry in the catalogue
 */
data class CatalogueEntry(
  /**
   * Type of entry
   */
  val type: CatalogueEntryType,

  /**
   * What provides the entry (core framework or plugin)
   */
  val providerType: CatalogueEntryProviderType,

  /**
   * Plugin name that provides the entry (may not be set for core entries)
   */
  val pluginName: String,

  /**
   * Key for the entry
   */
  val key: String,

  /**
   * Associated values for the entry
   */
  val values: Map<String, String> = mapOf()
)

/**
 * Type of provider for an entry in the catalogue
 */
enum class CatalogueEntryProviderType {
  CORE, PLUGIN
}
