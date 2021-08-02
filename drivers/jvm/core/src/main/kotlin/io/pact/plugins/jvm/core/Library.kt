package io.pact.plugins.jvm.core

class PactPluginNotFoundException(val name: String, val version: String?) :
    RuntimeException("Plugin $name with version ${version ?: "any"} was not found")

class PactPluginEntryFoundException(val type: String) :
  RuntimeException("No interaction type of '$type' was found in the catalogue")

class Library {
    fun someLibraryMethod(): Boolean {
        return true
    }
}
